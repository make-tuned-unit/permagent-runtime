# Ternary Bonsai 2 27B (PrismML) — review for the Permagent two-mini runtime
Date: 2026-09-20. Research + read-only inspection. No downloads, no installs, no service changes.

Tags: [VENDOR] = PrismML or its own docs/PR. [INDEPENDENT] = third party. [LOCAL-OBSERVED] = measured on this M4 today. [UNVERIFIED] = asserted by a secondary blog with no primary backing.

---

## PART A — WEB RESEARCH

### Sources consulted
- https://prismml.com/news/bonsai-2-27b [VENDOR]
- https://docs.prismml.com/bonsai-2-27b [VENDOR]
- https://docs.prismml.com/run/llamacpp [VENDOR]
- https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf (model card + raw README) [VENDOR]
- https://huggingface.co/prism-ml/Ternary-Bonsai-2-27B-gguf-dev [VENDOR, not fetched, listed in search]
- https://www.prnewswire.com/news-releases/prismml-launches-bonsai-2-27b-its-most-capable-model-yet-302882228.html [VENDOR press release]
- https://x.com/PrismML/status/2100692248480596348 [VENDOR]
- https://www.marktechpost.com/2026/09/18/prismml-releases-ternary-bonsai-2-27b-a-5-9-gb-apache-2-0-model-retaining-98-2-of-qwen3-8-27b-performance/ [secondary, most technically detailed]
- https://www.modemguides.com/blogs/modemguides-blog/run-bonsai-2-27b-locally ("three catches") [secondary]
- https://www.datacamp.com/blog/bonsai-2-27b [secondary]
- https://www.mindstudio.ai/blog/ternary-bonsai-2-27b-run-mac-locally [secondary]
- https://www.mindstudio.ai/blog/bonsai-2-27b-local-install [secondary, listed]
- https://atomic.chat/blog/guides/how-to-run-bonsai-2-locally [secondary]
- https://regolo.ai/bonsai-27b-explained-how-a-27b-model-runs-locally/ [secondary — note: this page is about **Bonsai 1** / Qwen3.6-27B, not Bonsai 2. Do not cite it for Bonsai 2 numbers.]
- https://news.ycombinator.com/item?id=49746618 [INDEPENDENT — HN thread]
- https://openrouter.ai/prism-ml/ternary-bonsai-2-27b [hosting]
- https://ollama.com/tobestyledintro/Ternary-Bonsai-2-27B [third-party Ollama upload — see caution below]
- https://github.com/john-rocky/coreai-model-zoo/issues/28 [community port request]
- https://github.com/PrismML-Eng/llama.cpp [the fork]

Note: WebSearch budget for the session was exhausted (200/200) partway through; r/LocalLLaMA was therefore not reachable directly. HN is the only independent community source captured here.

### A1. What it actually is

- Base: **Qwen3.8-27B**, 27.36B params, no structural change. [VENDOR]
- Method: **post-training quantisation (PTQ), not QAT and not distillation.** A **blockwise Hadamard rotation with block size 1024** is applied before ternary assignment; weights are stored in that rotated basis. Groups of 128 weights share one FP16 scale. ~1.71–1.72 effective bits/weight. [VENDOR via marktechpost]
- Higher-precision residue: only **26.2M params (0.0976 %)** stay unquantised — the **recurrent-state path and normalisation weights**. Embeddings + LM head are 2.54B params and are counted separately in the footprint. The **vision tower (0.47B) is a separate 0.63 GB Q8_0 mmproj file**, not ternary. [VENDOR via marktechpost]
- Architecture: **hybrid attention, ~75 % linear-attention layers / ~25 % full-attention layers.** This matters for KV cache. [VENDOR]
- Multimodal: yes, **text + image in**. mmproj loaded only when you pass images; text-only inference skips it. [VENDOR]
- Context: **262,144 tokens**. [VENDOR]
- Thinking: defaults to reasoning **on**. Per-request `thinking_budget_tokens` (0 = off, -1 = unlimited). Two inherited reasoning-effort settings: `xhigh` (default) and `medium`. [VENDOR]
- Chat template / tools: inherits Qwen3.8-27B formatting. OpenAI-style `tools` array in, `choices[0].message.tool_calls` out — "no prompt hacks required". llama.cpp path needs **`--jinja`** for tool_calls and **`--mmproj`** for images. [VENDOR]
- Sampling: thinking temp 1.0 / top_p 0.95 / top_k 20; instruct temp 0.7 / top_p 0.80 / top_k 20. (docs/run page separately suggests temp 0.5 / top_p 0.85 — the two vendor pages disagree.) [VENDOR]
- License: **Apache-2.0**, commercial use permitted, no extra restrictions found. [VENDOR]
- Relation to Bonsai 1: Bonsai 1 (Jul 2026) was derived from **Qwen3.6-27B** and offered binary {−1,+1} ~1.125 bpw / 3.9 GB and ternary ~1.71 bpw / 5.9 GB packs, retaining ~95 %. Bonsai 2 moves to the Qwen3.8 base, adds the Hadamard rotation, and claims 98.2 %. [VENDOR]

### A2. RUNTIME SUPPORT — the crux

This is the finding that decides everything.

- **The GGUFs do NOT load in mainline llama.cpp.** Quant type names are **`PTQ1_0`** (dense trits, 1.75 bpw, **5.95 GB**) and **`PQ2_0`** (2-bit slot per trit, 2.13 bpw, **7.21 GB**). Model card: *"Stock llama.cpp will not run these files"*; mainline *"rejects `PQ2_0` and `PTQ1_0` as unknown types"*. [VENDOR]
- **Worse: silent wrong output.** A `Q2_0`-named Bonsai file will *load* on a stock build and produce garbage without warning, because stock has no Hadamard activation-rotation support. [VENDOR + secondary]
- Required runtime: **PrismML fork `github.com/PrismML-Eng/llama.cpp`, branch `prism`, release tag `prism-b10658` or newer**, with prebuilt macOS arm64 binaries. **Metal is supported in the fork** (`-DGGML_METAL=ON`, default on macOS); CUDA and CPU too; no SYCL. [VENDOR]
- **llama-server works in the fork** and is OpenAI-compatible. [VENDOR]
- **Not upstream.** Docs say the rotated basis is "not yet upstream" and give no timeline. Bonsai **1**'s `Q1_0` and group-64 ternary *were* merged upstream, so PrismML has upstreamed before — but Bonsai 2's types have not. [VENDOR]
- **Ollama: no.** **LM Studio: no.** Neither is mentioned as supported; the "three catches" piece states flatly *"It does not run in Ollama or stock llama.cpp yet."* The `ollama.com/tobestyledintro/Ternary-Bonsai-2-27B` entry is a **third-party upload, not PrismML** — given the silent-garbage failure mode, treat it as untrustworthy until proven otherwise. [VENDOR/secondary]
- **MLX: stock mlx-lm, but a custom loader.** No forked MLX packages and no source build needed for the Python path — *but* you must load via the **`vision_artifact.load_vl_model` helper bundled in the model download**, because the weights use a custom `model_type` (`prism_hadamard_qwen35`). Generic MLX loaders **silently produce wrong output**. MLX package: `prism-ml/Ternary-Bonsai-2-27B-mlx-2bit`. [VENDOR/secondary]
- **MLX server: none documented.** No OpenAI-compatible server is described for the MLX path. It is a Python library call. → an MLX route means a **Python sidecar**, which we have explicitly ruled out.
- **iOS/iPad claims:** not substantiated in anything fetched for Bonsai 2. The regolo piece quotes 11 tok/s on iPhone 17 Pro but that is **Bonsai 1**. A community request to port it to Core ML exists (coreai-model-zoo issue #28) and is **unstarted**. [UNVERIFIED]
- **logprobs / `n_probs`:** **not documented anywhere** for the fork. llama-server generically supports it, and there is no obvious reason the fork would remove it, but nobody states it works — and logprob *calibration* on a 1.7-bit model is an open question in its own right. [UNVERIFIED]
- **llama.cpp RPC:** would become unnecessary for this model if it fits on one box — the fork still contains upstream's RPC code, but the whole point is that 5.95 GB fits in one mini.
- Hosted free: Together AI, Puter, OpenRouter. [VENDOR]

### A3. APPLE SILICON PERFORMANCE

There is **no published measurement on any base M1/M2/M3/M4 with 16 GB.** Everything below is a bigger chip.

| Chip | gen tok/s | prefill tok/s | source |
|---|---|---|---|
| M5 Max | 46.8–47 | — | [VENDOR] |
| M5 Pro | 28–29 (28.1 on PQ2_0) | 387 (PQ2_0) | [VENDOR] |
| M4 Pro | 18 | — | [VENDOR, flagged "pre-rotation build, pending re-measurement"] |
| M4 Pro (HN user) | ~15 @ 64k ctx | ~100 | [INDEPENDENT] |
| M-series range (HN) | 10 – 46.8 | — | [INDEPENDENT] |
| RTX 5090 | up to 143 | — | [VENDOR] |
| RTX 4090 | 91–97 | — | [VENDOR] |
| RTX 3070 | 40.6 | — | [INDEPENDENT] |

- **TTFT: no published figure anywhere.** [UNVERIFIED]
- **Peak memory: PrismML publishes no memory tables.** [VENDOR admission via secondary]
- MLX resident footprint ≈ **7.2 GB** for the LM component; guide calls 16 GB "the practical floor", recommends 32 GB. [secondary]
- **KV cache: ~64 KiB/token** claimed across the hybrid architecture, which at ~12 GB usable leaves roughly **56K tokens** of context after a 7.21 GB PQ2_0 model. [UNVERIFIED — single secondary source, and the number is large; must be measured.]
- **KV quantisation exists** via an env flag `BONSAI_KV4=1`, claimed ~3.5× cache reduction. `BONSAI_MMPROJ_CPU=1` frees ~0.9 GiB by moving the vision projector to system RAM. [UNVERIFIED — single secondary source]
- macOS GPU wired limit: the same guide states macOS caps GPU memory at **~75 % of unified memory** → **~12 GB usable on a 16 GB mini**. Consistent with the commonly cited 66–75 % range. [UNVERIFIED but plausible]
- HN: one contributor identified a **Metal tensor-API compilation issue depressing prefill**, implying published Apple prefill numbers may be understated (or that the shipped binaries are not well optimised). [INDEPENDENT]
- Ternary models are prefill-compute-bound; the only prefill datum for Bonsai 2 on Apple is 387 tok/s on an M5 Pro [VENDOR] and ~100 tok/s on an M4 Pro [INDEPENDENT]. A base M4 will be below the M4 Pro figure.

### A4. QUALITY, INDEPENDENTLY

**Vendor headline:** aggregate **83.9 vs 85.4 = 98.2 %**, 20 benchmarks, thinking mode, measured with EvalScope + vLLM on an H100. (A second vendor figure says 84.78 across 14 thinking-mode benchmarks — the two are not reconciled.) [VENDOR]

Per-category [VENDOR, via datacamp]:

| Benchmark | Bonsai 2 | Qwen3.8-27B FP16 | delta |
|---|---|---|---|
| AIME26 | 95.83 | 94.58 | +1.25 |
| MATH-500 | 98.80 | 99.80 | −1.00 |
| HumanEval+ | 95.12 | ~94 | + |
| LiveCodeBench | 90.07 | ~90 | ≈ |
| IFEval | 91.31 | ~91 | + |
| **BFCL v3 (tool use)** | **74.92** | **76.74** | **−1.82** |
| τ²-bench + BFCLv3 agentic | 77.57 | 79.74 | −2.17 |
| MMLU-Redux | 89.09 | 91.46 | −2.37 |
| MMMU-Pro (vision) | 75.49 | 81.73 | −6.24 |
| OCRBench v2 | 56.88 | 60.99 | −4.11 |

**The "three catches"** (modemguides) [secondary, the single most useful critical source]:
1. **Runtime**: needs the fork or the bundled MLX loader; no Ollama, no stock llama.cpp.
2. **Quality varies enormously by task.** The 98.2 % is at **maximum reasoning effort**. At **medium effort** — the setting you would actually use on a laptop — retention drops to **96.0 %**. And on **agentic coding benchmarks (Terminal-Bench 2.1, SWE-bench Verified) it keeps only about 75 % of full-precision performance.** That is the number that matters to us, and it is ~25 points of relative loss hidden inside a "98.2 %" headline.
3. **Every metric is vendor-reported.** At launch *"no one outside PrismML had published a Bonsai 2 measurement."*

**HN thread (independent)** [INDEPENDENT]:
- Recitation test: "Jabberwocky" degrades after the first stanza vs full precision → **rare-token / memorised-sequence damage**, the classic extreme-quantisation failure.
- WebGPU demo showed **immediate looping**.
- Multiple users: **"struggles with longer context"**, **"goes into loops"** in extended reasoning — i.e. **long-context drift**, confirmed by more than one person.
- Real coding task: 25 minutes on a simple Swift app that GPT-5.6 did in 3.
- Accusation that benchmarks are **cherry-picked to avoid hard multi-turn reasoning**.
- Several users report **Qwen3.8-27B at Q4 performs comparably or better with better reliability**.
- ISTA 3-bit quant: similar compression, worse than claimed in practice.
- Friction over the fork; requests to upstream; no optimised kernels for several platforms.

**Realistic alternatives on our hardware**, as of Sept 2026:
- **Qwen3.8-27B Q4_K_M split over two minis** — what we run now. ~17.8 GB file (measured locally), ~9.3 GB wired on the M4 half, ~5 tok/s (our own code comments). Full fidelity, mainline llama.cpp, works today.
- **Qwen3.x 8B/14B at Q4 on one mini** — ~5–9 GB, mainline everything, no fork, but 9B-class open-weight tool calling was measured at ~72 % in our own harness work, which is the known ceiling.
- **Gemma-4 class** — already in our FEATURED_MODELS as `unsloth/gemma-4-E4B-it-GGUF:Q4_K_M` and `unsloth/gemma-4-26B-A4B-it-GGUF:Q4_K_M`, both marked `native_tool_calling: true` and both with mmproj. The 26B-A4B is an MoE with 4B active — that is arguably the closest real competitor to Bonsai 2 for our purpose and it runs on **stock llama.cpp today**.
- **MiniCPM5-2B** — `minicpm5-2b-q8` is already pulled locally (2.7 GB).
- Bonsai 1 ternary/binary — already upstream in llama.cpp, so loadable today, but a Qwen3.6 base and ~95 % retention.

### A5. Vendor credibility

- PrismML is a compression-focused lab; Bonsai 1 shipped ~2 months before Bonsai 2 (fast cadence). Weights are genuinely **Apache-2.0** and the **kernels are open** (the fork is a public GitHub repo). That is materially better than a closed-weights claim.
- Track record is real but thin: Bonsai 1's `Q1_0`/group-64 ternary types **did land upstream in llama.cpp**, which is evidence PrismML follows through on upstreaming eventually.
- **Red flags:** every Bonsai 2 number is self-reported; no memory tables published; two different aggregate figures (83.9/20-bench and 84.78/14-bench) in vendor materials; two different recommended sampling settings across vendor pages; the headline retention is measured at max reasoning effort while the practically relevant effort level and the agentic-coding retention are both much worse and are only disclosed in a third-party write-up; and the silent-wrong-output failure mode on stock runtimes is a serious footgun that they ship into an ecosystem full of re-uploaders.

---

## PART B — LOCAL READ-ONLY INSPECTION (the M4 workstation)

All [LOCAL-OBSERVED], 2026-09-20.

### Hardware / OS
```
hw.memsize            17179869184   (16 GiB)
machdep.cpu.brand     Apple M4
hw.model              Mac16,10
macOS                 26.2 (25C56)
```

### GPU wired limit
```
iogpu.wired_limit_mb: 0            # 0 = system default, no override set
debug.iogpu.wired_lwm_mb: 0
debug.iogpu.dynamic_lwm: 1
debug.iogpu.metal4_disable_async_mapping: 0
```
No manual raise of the wired limit. Default applies → ~11–12 GB usable by Metal.

### Memory right now — this is the headline local fact
```
vm_stat: page size 16384
  Pages free           4001        (~62 MB)
  Pages active       127522
  Pages inactive     115024
  Pages speculative   11586
  Pages wired down   521331        (~8.0 GB wired)
  Pages occupied by compressor 234548  (~3.6 GB)
memory_pressure: System-wide memory free percentage: 25%
vm.swapusage: total 14336.00M  used 13245.50M  free 1090.50M
```
Running the split script's own honest availability formula:
```
$ qwen38-split.sh memcheck
REFUSE — only 0MB genuinely available [available 0MB (need 9500MB),
 compressor holding 3198MB, swap 13245/14336MB used with 1090MB free,
 pressure level 2]; biggest holders: claude(422MB), claude(349MB),
 claude(298MB), com.apple.WebKit.WebContent(238MB), mediaanalysisd(188MB)
```
**Zero MB genuinely available, 13.2 of 14.3 GB swap in use, memory pressure level 2.** The M4 as it sits today cannot host *any* multi-GB model — not the 9.3 GB split half, and not a 6–8 GB Bonsai either — without first shedding load.

### Disk
```
/System/Volumes/Data   460Gi  400Gi used  23Gi avail  95% capacity
```
**23 GiB free.** A PTQ1_0 (5.95 GB) + mmproj (0.63 GB) download plus a fork build tree is roughly 8–12 GiB — that fits, but takes the machine to ~97 % full. `/Users/j/.local/models` already holds 18 GB (the Qwen3.8 Q4_K_M 17.8 GB + 1.7 GB MTP draft).

### Installed inference stacks
```
/opt/homebrew/bin/llama-server   version 0.1.1-dev (build 10470, commit 34af94cd9)
/opt/homebrew/bin/llama-cli      same
brew llama.cpp 10470
/opt/homebrew/bin/ollama         ollama version 0.32.9
mlx_lm.server                    NOT INSTALLED
python3                          /opt/homebrew/bin/python3, /usr/bin/python3
```
**Installed llama.cpp is build 10470. The PrismML fork base is `prism-b10658`.** Our build is ~190 builds *behind* the fork's base — so it is both mainline (no ternary kernels) and older. It **cannot** load `PTQ1_0`/`PQ2_0`, and would silently mis-run a `Q2_0`-named file.

Source checkout at `/Users/j/.local/src/llama.cpp` exists, HEAD `04b56914` (2026-08-18, "common: share thread pools when n_threads differ (#27138)"), but **`build/bin` does not exist** — the checkout is not built. The split script points `LLAMA_BIN` at `/Users/j/.local/src/llama.cpp/build/bin`, so the nightly split is currently relying on a binary directory that is absent (worth flagging separately from this review).

### Running processes
```
ollama serve (pid 1223, since Mon 11AM)
ollama's bundled llama-server (pid 86487) serving a model on 127.0.0.1:54816
   flags: -c 4096 -np 1 --no-jinja --chat-template chatml
          --cache-type-k q8_0 --cache-type-v q8_0 --flash-attn on
          -b 512 -ub 512 --context-shift --keep 4
   RSS ~4.35 GB (25.9% of RAM)
```
No `rpc-server`, no `mlx` process. The nightly split is not up (correct — it is 8 PM).

### launchd
```
ai.permagent.qwen38-split.preflight   01:45
ai.permagent.qwen38-split.start       01:50, 02:20, 02:50, 03:20, 03:50, 04:20 (retry slots)
ai.permagent.qwen38-split.stop        06:10
ai.permagent.daemon                   pid 90460
com.picker.stockscanner               pid 21155  (the :8080 Flask scanner)
com.polybot.nearcertain                pid 1230
homebrew.mxcl.ollama                  pid 1223
```
No tokens or secrets were present in the three split plists (they only set PATH and HOME). I did not open any env files.

### The nightly split, exactly as configured
`/Users/j/.local/llama/qwen38-split.sh` (22 KB, heavily documented):
- `PORT=8081` (was 8080; moved 2026-08-25 after colliding with the Picker's Flask scanner). This script is the single definition of that port.
- `MODEL=/Users/j/.local/models/qwen38-gguf/Qwen3.8-27B-Q4_K_M.gguf` (17.8 GB) + `mtp-Qwen3.8-27B-Q4_0.gguf` (1.7 GB draft/MTP).
- Second half runs `ggml-rpc-server` on the **M1** (`jessesharratt@100.74.232.95` over Tailscale for ssh; the RPC link itself uses the M1's **169.254 link-local** address, re-discovered each start because it changes on every reboot).
- Memory preflight: needs **9500 MB** genuinely available, ≥2048 MB swap free, ≤6144 MB swap used. Refuses otherwise and names the top RSS holders.
- Readiness is a **three-stage** check: port listening → `/v1/models` returns a `data` array → a 1-token `/v1/chat/completions` returns a `choices` array (proving the RPC hop works). Budget 900 s.
- Supervisor subprocess traps EXIT/TERM/INT so every death writes a `stop:` line.
- Logs in `/Users/j/.local/llama/logs/`.

**Retiring the split would mean deleting/unloading 3 launchd jobs, the 22 KB script, the M1 ssh+rpc dependency, the 169.254 IP discovery, the memory preflight, the 3-stage readiness probe, and ~19.5 GB of GGUF — and in `librarian.rs`, the whole `probe_dedicated_endpoint` / `split_refusal_reason` apparatus (roughly lines 1780–2250).**

### The Rust runtime
- `crates/goose/Cargo.toml`: `llama-cpp-2 = "0.1.151"` with features `["sampler","mtmd"]`, and `["sampler","metal","mtmd"]` on macOS. Resolved in `Cargo.lock` to **`llama-cpp-2` / `llama-cpp-sys-2` 0.1.154**, both from crates.io (checksummed, not a git dep).
- `llama-cpp-sys-2` **vendors its own llama.cpp source tree** (`~/.cargo/registry/src/.../llama-cpp-sys-2-0.1.154/llama.cpp/`). A custom quant type therefore cannot be added by bumping a version — it needs the **sys crate itself forked** to vendor PrismML's llama.cpp, plus Rust-side enum/type additions. That is a real fork of a dependency, not a config change.
- `crates/goose/src/providers/local_inference.rs` (718 lines): `InferenceRuntime` singleton, `LlamaModel::load_from_file`, `recommend_local_model()` picks **the largest featured model whose `size_bytes` fits `available_inference_memory_bytes()`** (max free memory over GPU/accelerator devices, else CPU).
- **Tool calling in-process is emulated, always.** Line ~549: *"(native tool-calling templates, tool grammars, tool-call parsing — utilityai/llama-cpp-rs#1037), so ALL requests with tools run through the text-based emulator, even for models whose `native_tool_calling` is set."* So the in-process path would throw away Bonsai 2's native Qwen-style tool calling.
- `crates/goose/src/providers/local_inference/local_model_registry.rs` — `FEATURED_MODELS`:
  | spec | native_tool_calling | mmproj |
  |---|---|---|
  | `bartowski/Llama-3.2-1B-Instruct-GGUF:Q4_K_M` | false | — |
  | `bartowski/Llama-3.2-3B-Instruct-GGUF:Q4_K_M` | false | — |
  | `bartowski/Hermes-2-Pro-Mistral-7B-GGUF:Q4_K_M` | false | — |
  | `bartowski/Mistral-Small-24B-Instruct-2501-GGUF:Q4_K_M` | false | — |
  | `unsloth/gemma-4-E4B-it-GGUF:Q4_K_M` | true | mmproj-BF16.gguf |
  | `unsloth/gemma-4-26B-A4B-it-GGUF:Q4_K_M` | true | mmproj-BF16.gguf |
- `crates/goose/src/providers/ollama.rs` (1979 lines): default model `qwen3`, port 11434, `is_local_host` drives `data_locality` (a remote Ollama counts as cloud egress). 10 retries with 1.5× backoff because big models 500 while loading.
- **No `n_probs` / `logprobs` request plumbing exists anywhere in `crates/goose/src/`.** The only hits are a passive `logprobs: Option<Vec<Value>>` field in the OpenAI Responses format and `logprobs: null` inside test fixtures. A logprob-based decision backend is **unbuilt on our side regardless of model**.
- `ollama list` locally: `qwen2.5-coder:7b` (4.7 GB), `minicpm5-2b-q8` (2.7 GB), `qwen25-16k` (4.7 GB), `qwen2.5:7b` (4.7 GB).
- Librarian model comes from `librarian_schedule.json` (`model` key), default window "02:00 + 240 min".
- Our own code comment: *"The split 27B does ~5 tok/s on the minis"* (librarian.rs ~line 2384). [LOCAL-OBSERVED, from our own instrumentation]

---

## PART C — VERDICT

### (i) Librarian: could Bonsai 2 on one mini replace the two-mini split?

**In principle yes, and it is by far the most attractive of the three uses — but it is blocked today and the quality question is answerable only by measurement.**

The fit is genuinely good. The Librarian's job is short-prompt, short-output, batch, overnight, non-agentic: ~700-token prompts → ~150 tokens, supersession adjudication, cluster consolidation. These are exactly the tasks where a ternary model's damage profile (agentic coding −25 %, long-context drift, rare-token recitation) is **least** likely to bite, and where the near-parity categories (IFEval 91.31, MMLU-Redux 89.09) are what is actually exercised. The Librarian also does not need vision, so the 0.63 GB mmproj stays unloaded and the −6 MMMU-Pro hit is irrelevant.

The prize is large and concrete:
- **Retires the M1 dependency entirely** — no ssh, no 169.254 IP rediscovery each reboot, no Tailscale hop, no RPC link that can silently lose a node mid-stream.
- **Retires the 01:50–06:10 window** — a 5.95 GB model that loads in seconds can run *whenever*, not only when the machine is idle at night. That alone removes the retry-slot machinery, the preflight refusals, and the class of "no split tonight" failures.
- **Retires the :8081 allocation** and its history of colliding with the Picker's :8080 scanner.
- **Frees ~19.5 GB of disk** (Q4_K_M + MTP) against a volume that is 95 % full.
- **Throughput**: the split does ~5 tok/s [LOCAL-OBSERVED]. A base M4 running a 5.95 GB ternary model should be well above that — the M4 *Pro* datum is 15–18 tok/s [VENDOR/INDEPENDENT], so a base M4 plausibly lands 8–14 tok/s. **A 2–3× speedup is the realistic expectation, not the 28 tok/s M5 Pro headline.**

Expected quality delta: **small but unmeasured**. Best guess is a low-single-digit degradation on description/categorisation quality and a slightly higher rate of borderline supersession calls. That is acceptable *if* it holds. It has to be measured against the current split's own outputs, not against a benchmark.

**Blocker: the fork.** Running this means running a third-party fork of llama.cpp as a nightly unattended service. That is a supply-chain and maintenance commitment we do not currently have anywhere else in the stack, and the fork's base (b10658) is ahead of our installed build (10470), so it is not a drop-in swap of a binary.

### (ii) Daytime worker tier

**No.** Two independent reasons, either sufficient:

1. **The RAM is not there.** The M4 today shows **0 MB genuinely available and 13.2/14.3 GB swap used** [LOCAL-OBSERVED]. A 5.95 GB model plus even a modest KV cache does not fit alongside Rust builds, the Tauri app, the daemon, Ollama's already-resident 4.35 GB llama-server, and a browser. The daytime machine is the constraint, not the model.
2. **Tool-calling reliability is the gating quality, and this is where the model is weakest relative to its headline.** BFCL v3 **74.92** [VENDOR] is *below* the ~72 % 9B-class figure only by a hair — it is not a step change. And the agentic-coding retention of **~75 % of full precision** on Terminal-Bench/SWE-bench [secondary], plus independent reports of **looping in extended reasoning** and **long-context struggles** [INDEPENDENT HN], are precisely the failure modes that kill an agentic loop. Our existing harness finding — local models cannot reliably drive an agentic coding loop — is *reinforced*, not overturned, by this model.

For a daytime local worker, `unsloth/gemma-4-26B-A4B-it-GGUF:Q4_K_M` (already in FEATURED_MODELS, `native_tool_calling: true`, runs on stock llama.cpp) is the better bet and costs nothing new.

### (iii) Decision backend (logprobs over option tokens)

**No — a 27B is the wrong shape for this, and the path is doubly blocked.**

- Logprob support in the fork is **undocumented** [UNVERIFIED].
- Logprob **calibration** on 1.72-bit weights is a genuine open question; extreme quantisation is known to distort the tail of the distribution, and a typed yes/no decision reads exactly that.
- **We have no logprob plumbing at all** — `grep` finds no `n_probs`/`top_logprobs` request path anywhere in `crates/goose/src/` [LOCAL-OBSERVED]. The work is on our side first.
- A typed yes/no or pick-one decision is a sub-1B job. `Llama-3.2-1B-Instruct:Q4_K_M` is already in FEATURED_MODELS, loads in-process, costs ~800 MB, and can be logprob-read through mainline llama-server today. Spending 6 GB and a forked runtime on it would be absurd.

### (iv) Integration path, ranked by effort

1. **PrismML llama-server fork over HTTP** — *lowest effort, and the only viable path.* Prebuilt macOS arm64 binary from the `prism-b10658+` release, run on a port, point `PERMAGENT_LIBRARIAN_ENDPOINT` at it. **Zero Rust changes** — `librarian.rs` already speaks OpenAI-compatible SSE to a llama-server and already has the three-stage readiness probe. Needs `--jinja` for tool_calls, explicit `-c` (the docs warn `-c 0` will exhaust memory), `-ngl 99`, `-fa on`. Cost: accepting a forked binary as a service.
2. **MLX sidecar** — *blocked by policy.* Stock mlx-lm but a **bundled Python loader** with a custom `model_type`, and **no documented OpenAI-compatible server**. That is a Python sidecar, which we have explicitly ruled out. Also nothing is installed (`mlx_lm.server` absent).
3. **In-process GGUF provider** — *blocked, and the worst option even if unblocked.* `llama-cpp-sys-2` **vendors its own llama.cpp tree**, so supporting `PTQ1_0`/`PQ2_0` requires forking the sys crate to vendor PrismML's source plus Rust-side type additions — not a version bump. And the in-process path **forces every tool call through the text emulator** (llama-cpp-rs#1037), discarding the native tool calling that is one of the model's selling points.

**Blocked today, plainly:** mainline llama.cpp (ours is build 10470, mainline, no ternary kernels), Ollama, LM Studio, our in-process provider, and any logprob use. The *only* unblocked route is the forked `llama-server` over HTTP.

### (v) BENCH PLAN — minimal, for later approval

**Prerequisites (must be true before starting):**
- **Free ≥ 25 GiB disk.** Currently 23 GiB free at 95 %. Downloading 5.95 GB + 0.63 GB + a fork build tree on top of that is imprudent. Deleting the 1.7 GB MTP draft is not enough; plan to reclaim ~15 GiB first.
- **Run on the M1, not the M4.** The M4 is at zero available memory and heavy swap. The headless M1 is idle outside the window, is the machine the split would free anyway, and isolates the test from the dev machine.
- Download: **6.6 GB** (PTQ1_0 5.95 GB + mmproj 0.63 GB, skip mmproj if text-only → **5.95 GB**). Optionally PQ2_0 7.21 GB for comparison. Disk needed including the fork build: **~15 GiB**.

**Step 0 — acquire (no install into the system).**
```
# fork binary, prebuilt, into a sandbox dir — do not put it on PATH
mkdir -p ~/.local/bonsai2 && cd ~/.local/bonsai2
# download prism-b10658+ macOS-arm64 release tarball from
#   https://github.com/PrismML-Eng/llama.cpp/releases
# verify checksum against the release page before extracting
huggingface-cli download prism-ml/Ternary-Bonsai-2-27B-gguf \
  Ternary-Bonsai-2-27B-PTQ1_0.gguf --local-dir ~/.local/bonsai2/models
```

**Step 1 — serve, on a port that collides with nothing (8080=Picker, 8081=split).**
```
~/.local/bonsai2/bin/llama-server \
  -m ~/.local/bonsai2/models/Ternary-Bonsai-2-27B-PTQ1_0.gguf \
  --port 8082 --host 127.0.0.1 \
  -ngl 99 -fa on -c 8192 --jinja \
  --temp 0.7 --top-p 0.80 --top-k 20 \
  --metrics 2>&1 | tee ~/.local/bonsai2/serve.log
```
Repeat at `-c 32768` and `-c 131072` for the KV-cache measurement. Also once with `BONSAI_KV4=1` set.

**Step 2 — sanity, before any timing.** Confirm the model is not silently broken: ask it to recite a well-known passage, ask a factual question, and ask for strict JSON. **Go/no-go: coherent, non-looping output on all three, and valid JSON on 10/10 attempts.** If it loops (as HN reported in the WebGPU demo), stop here.

**Step 3 — throughput / TTFT / memory.** Use llama-server's own timings, 20 runs, discard first 2.
- Prefill tok/s and generation tok/s at 700-token and 8000-token prompts.
- **TTFT** from the SSE stream's first token.
- **Peak RSS** via `ps -o rss= -p <pid>` sampled at 1 Hz, and Metal wired footprint via `vm_stat` delta.
- **Go/no-go for the Librarian: ≥ 10 tok/s generation** (2× the split's measured ~5 tok/s) **and peak RSS ≤ 11 GB at `-c 32768`.** Below 10 tok/s the fork is not worth its maintenance cost; above 11 GB it does not leave a 16 GB mini usable.

**Step 4 — Librarian agreement, the decisive test.** Take **50 real memories** already processed by the current split (their outputs are on record). Re-run the same three Librarian prompts through :8082:
- describe/categorise (~700 tok in → ~150 tok out),
- supersession adjudication (pairs where the split said supersede and pairs where it said not),
- cluster consolidation.
Score by **blind A/B**: for each memory, present the split's output and Bonsai's output unlabelled and pick the better or "equivalent".
- **Go/no-go: Bonsai wins-or-ties ≥ 85 % of the 50 on describe/categorise, AND agrees with the split on ≥ 90 % of supersession calls.** Supersession is the one with destructive consequences, so it carries the stricter bar.

**Step 5 — tool calling, only if step 4 passes.** 30 single-turn and 10 multi-turn tool tasks drawn from our harness's existing tool-call set, via `--jinja` and the OpenAI `tools` array.
- **Go/no-go for a daytime worker tier: ≥ 90 % single-turn well-formed-and-correct.** Given BFCL v3 74.92 [VENDOR], expect this to fail; run it to have our own number rather than a vendor's.

**Step 6 — logprob sanity, optional and cheap.** `POST /completion` with `"n_probs": 5` on ten forced binary choices with known answers.
- **Go/no-go: `n_probs` is returned at all** (undocumented for the fork), **and** the correct option carries higher logprob on ≥ 9/10. Expect to abandon this and use a 1B on mainline instead.

**Total cost:** ~6 GB download, ~15 GiB disk, one evening on the idle M1, no changes to the M4 and no changes to the running split.

### (vi) Risks and unknowns, plainly

**Risks**
- **Forked-runtime dependency.** Running an unattended nightly service on a third-party llama.cpp fork with no upstreaming timeline. If PrismML stops maintaining it, we are pinned to `prism-b10658` forever and lose every upstream llama.cpp fix.
- **Silent wrong output.** The defining footgun: mis-named files load on stock builds and emit garbage *without warning*. Any third-party re-upload (including the Ollama one) is a trap. The step-2 sanity gate exists solely for this.
- **Every quality number is the vendor's.** No independent benchmark rerun existed at launch, and none surfaced in this research.
- **Disk.** 23 GiB free at 95 % capacity. A careless download leaves the machine wedged.
- **The M4 has no memory headroom at all right now** — 0 MB available, 13.2 GB swap used. Benching on it would be misleading at best and destabilising at worst.
- **Long-context drift and looping** independently reported by more than one HN user. The Librarian's prompts are short, which mitigates it, but cluster consolidation can grow.

**Unknowns**
- **No base-M1/M2/M3/M4 16 GB measurement exists anywhere.** Every published Apple number is a Pro or Max. Our extrapolation to 8–14 tok/s on a base M4 is a guess.
- **TTFT: unpublished.**
- **Peak memory: PrismML publishes no tables at all.**
- **KV cache ~64 KiB/token** and the `BONSAI_KV4` / `BONSAI_MMPROJ_CPU` flags rest on **one secondary blog**. Unconfirmed by the vendor docs.
- **logprobs/`n_probs` in the fork: undocumented.** And calibration at 1.72 bpw is an open research question, not just an API question.
- **Upstreaming timeline: none given.** Bonsai 1's types did land upstream, so it may happen — and **if it does, this whole analysis changes**, because the only real blocker disappears.
- The two conflicting vendor aggregate figures (83.9/20 vs 84.78/14) and two conflicting sampling recommendations suggest the vendor materials were assembled quickly.

**One-line recommendation:** Do not adopt it as a daytime worker or a decision backend. Do keep it as a **live candidate for the Librarian only**, benched on the idle M1, gated on step 2 (not-broken) and step 4 (agreement with the current split) — and **re-evaluate immediately if PrismML upstreams `PTQ1_0`/`PQ2_0` into mainline llama.cpp**, which would remove the single largest objection.
