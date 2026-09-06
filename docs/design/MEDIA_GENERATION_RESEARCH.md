# Media generation for Permagent — implementation-grade research

**Deep-research deliverable, compiled 2026-09-01. Untracked; do not commit.**
No code was changed and nothing was built to produce this document.

**Who this is for:** whoever wires image generation, video generation, and document authoring into the
Permagent agent. It answers, in order: *what can we bundle and run ourselves*, *what must we buy from a
cloud API and at what price*, *what should the agent's tools look like*, and *what will bite us*.

**Provenance labels.** Every load-bearing number is tagged:

- **[VENDOR]** — stated on an official docs, pricing, model-card, or license page.
- **[MEASURED]** — a real measurement by a named party on named hardware (GitHub issue, benchmark page).
- **[SOURCE]** — read first-hand out of source code or crate/lockfile metadata on this machine.
- **[DERIVED]** — arithmetic on [VENDOR] numbers.
- **[UNVERIFIED]** — widely repeated but not confirmed against a primary source. Treat as a lead, not a fact.
- **[OURS]** — a judgement this document is making. Defensible, but ours.

Do not launder an [OURS] or [UNVERIFIED] figure into a plan as "what the vendor says".

---

## 0. Our starting position

Everything below is [SOURCE] — read out of this repo and out of `~/.cargo/registry` on 2026-09-01.

| Fact | Value | Where |
| --- | --- | --- |
| Daemon | Rust workspace, `crates/goose*` | `Cargo.toml` |
| Desktop shell | Tauri 2 (`features = ["unstable"]`), wry 0.55 → **WKWebView**, not Chromium | `ui/desktop/src-tauri/Cargo.toml` |
| Distribution | DMG + `.app`, Developer ID signed, self-updating. Not Mac App Store. | `docs/design/APPLE_LIQUID_GLASS_RESEARCH.md` §0 |
| Already-shipped binaries | `permagentd` 329 MB, `permagent` 243 MB, `libonnxruntime.dylib` 27 MB, `libsherpa-onnx-c-api.dylib` 3.9 MB | `ui/desktop/src-tauri/binaries/` |
| Already-shipped resources | 217 MB total — `whisper/whisper-base-q8_0.gguf` **74 MB**, `mcp-runtime` 142 MB | `ui/desktop/src-tauri/resources/` |
| Bundled-model precedent | one 74 MB quantized GGUF ships inside the DMG | same |
| Downloaded-model precedent | Kokoro TTS lives in `dirs::data_dir()/permagent/models/voice/`, fetched after install | `crates/goose-server/src/voice/ort_kokoro_backend.rs:820` |
| House ML pattern | **Apache-2.0 open weights + ONNX + `ort` crate + no Python + "GPL-clean"** | `ort_kokoro_backend.rs:1` — *"Standalone Kokoro TTS via ort + misaki-rs (GPL-clean shipping backend) … No sherpa-onnx dependency in this path."* |
| Measured local-model perf precedent | Kokoro TTS **0.24× realtime on Apple Silicon, CPU provider, release build** | same file, line 7 |
| Inference crates present | `candle-core/nn/transformers` 0.11 (`metal` feature), `llama-cpp-2` 0.1.151 (`metal`), `ort` 2.0.0-rc.13 | `crates/goose/Cargo.toml`, `Cargo.lock` |
| PDF today | `lopdf` 0.41 — **read only** (text extraction) | `crates/goose/src/reader/pdf.rs`, `crates/goose-mcp/src/computercontroller/pdf_tool.rs` |
| DOCX today | `docx-rs` 0.4.20 — **already writes** (append / replace / structured insert / add image / styles) | `crates/goose-mcp/src/computercontroller/docx_tool.rs` |
| XLSX today | `xlsx_tool.rs` exists | `crates/goose-mcp/src/computercontroller/` |
| Charts today | `autovisualiser` MCP server renders **HTML** templates — chart, chord, donut, map, mermaid, radar, sankey, treemap — 3.9 MB of bundled JS assets | `crates/goose-mcp/src/autovisualiser/templates/` |
| Image generation today | **none.** No `generate_image`, no provider, nothing in the capability inventory. Greenfield. | `grep` over `crates/`, `ui/command-center/src` |
| UI type stack | Inter + Manrope (both OFL), referenced by name, **no font files bundled** | `ui/command-center/src/styles/tokens.ts:35` |
| macOS API reach from Rust | `objc2` 0.6.4, `objc2-app-kit` 0.3.2, `objc2-web-kit` 0.3.2 already in the Tauri lockfile via wry | `ui/desktop/src-tauri/Cargo.lock` |

Five consequences follow immediately, and they shape every recommendation below.

1. **We have a house pattern for shipping open weights and it works.** Kokoro TTS is Apache-2.0 weights,
   run through `ort`, with the GPL-encumbered dependency (espeak) deliberately engineered out. Media
   generation should look like that, not like a Python sidecar.
2. **"GPL-clean" is an existing, enforced constraint in this codebase.** That single-handedly disqualifies
   several otherwise-obvious options (Pandoc, ComfyUI, Draw Things-as-bundled-dependency).
3. **We are not Chromium.** Every HTML-to-PDF idea has to survive WKWebView, and §3 shows WKWebView's
   PDF API is not what people assume it is.
4. **We already write DOCX and already render charts as HTML.** Document production is not greenfield;
   it is half-built, in two mutually incompatible directions.
5. **The bundle is already ~1.1 GB of binaries and resources.** A "sub-2 GB media model budget" is not
   free headroom on a small app — it would roughly double the download.
---

## 1. Image generation

Read this section in the order it is written: what we can ship ourselves first, cloud second, Apple third.
That is the order Jesse asked for and it is also the order the evidence supports — the open-weights
licensing situation genuinely improved in 2026, and it changes the answer.

### 1.1 The bundleable tier — open weights we could legally ship

The gating question is *"may these weights be redistributed inside a closed-source commercial DMG?"*
Not "is it open." Here is the 2026 answer.

| Model | Params | License | **Ship in our DMG?** |
| --- | --- | --- | --- |
| **FLUX.2 [klein] 4B** | 4B | **Apache-2.0** [VENDOR] (HF API `license: apache-2.0`, model card: "fully open under Apache 2.0") | **YES — clean** |
| **Z-Image-Turbo** | 6B | **Apache-2.0** [VENDOR] | **YES — clean** |
| **FLUX.1 [schnell]** | 12B | **Apache-2.0** [VENDOR] — card says "personal, scientific, and commercial purposes" | **YES legally**, no practically (§1.1.2) |
| **Qwen3-4B** (text encoder both candidates need) | 4B | Apache-2.0 [VENDOR] | **YES — clean** |
| Qwen-Image | 20B | Apache-2.0 [VENDOR] | Legally yes; will not run on 16 GB |
| FLUX.2 [klein] 9B | 9B | FLUX Non-Commercial License [VENDOR] | **NO** |
| FLUX.1 / FLUX.2 [dev] | 12B / — | FLUX.1 [dev] Non-Commercial License [VENDOR]; commercial self-host is a paid BFL tier, price unpublished | **NO** without a paid deal |
| Ideogram 4 (9B) | 9B | "Ideogram 4 Non-Commercial" [VENDOR] | **NO** — and it is the best open model |
| SD 3.5 Medium / Large | — | Stability Community License: free commercially **under $1M annual revenue**, above which "any licenses granted to You under this Agreement shall terminate" [VENDOR] | **CONDITIONAL** — revocable, revenue cliff, registration required |
| SDXL-Turbo / SD-Turbo | 3B / 0.9B | **`sai-nc-community` — Stability AI *Non-Commercial* Research Community License**; the card points commercial users at a paid membership [VENDOR, re-verified §1.1.5] | **NO** — settled, and settled against us |
| SDXL 1.0 / SD 1.5 | — | CreativeML Open RAIL(++)-M | **CONDITIONAL** — see below |

**The OpenRAIL pass-through obligation, concretely.** OpenRAIL-M permits redistribution including hosted
use, *provided* you (a) reproduce the Attachment A use-based restrictions as an **enforceable provision of
your own EULA**, and (b) notify downstream users the model is subject to them [VENDOR]. That is a real
change to Permagent's legal text, not a checkbox. Given that two Apache-2.0 models are available and
better, there is no reason to take on OpenRAIL.

**[OURS] The licensing verdict: FLUX.2-klein-4B and Z-Image-Turbo are the only models that are
simultaneously (a) redistributable in a commercial bundle, (b) current-generation, and (c) capable of
running on 16 GB.** That is a materially better position than 2025, when the honest answer was "SD 1.5
under OpenRAIL, or nothing."

#### 1.1.1 Size — and why the sub-2 GB bundle budget is dead

Every 2026-era model is a diffusion transformer **plus a separate LLM text encoder**, and the encoder is
half the footprint. Both bundleable candidates use **Qwen3-4B** as their text encoder — 2.08–2.29 GB at
Q3/IQ4 on its own [VENDOR, HF file sizes].

| Stack | DiT | Text encoder | VAE | **Total working set** |
| --- | --- | --- | --- | --- |
| **FLUX.2-klein-4B Q3_K_S** | 2.10 GB | 2.08 GB | 0.17 GB | **~4.4 GB** |
| **FLUX.2-klein-4B Q4_K_S** | 2.58 GB | 2.29 GB | 0.17 GB | **~5.0 GB** |
| **Z-Image-Turbo Q2_K** | 2.59 GB | 2.08 GB | ~0.10 GB | **~4.8 GB** |
| **Z-Image-Turbo Q4_0** | 3.68 GB | 2.29 GB | ~0.10 GB | **~6.1 GB** |
| FLUX.1-schnell Q4_K_S | 6.78 GB | ~2.5 GB (T5-XXL) + 0.25 GB (CLIP-L) | 0.17 GB | **~9.7 GB** |
| SD 1.5 fp16 | 1.72 GB | 0.25 GB | 0.17 GB | ~2.14 GB |

All [VENDOR] — file sizes read from the HuggingFace API on the `city96`, `leejet`, `unsloth` and official
repos.

**[OURS] Only SD 1.5 fits a 2 GB bundle, and SD 1.5 in 2026 is not a credible default.** Plan for a
**~4.5–5 GB first-run download**, not a bundled model. We already have the pattern: Kokoro TTS lands in
`dirs::data_dir()/permagent/models/voice/` after install. Media models go next door in
`models/media/`. The 74 MB whisper GGUF stays the only model in the DMG.

#### 1.1.2 Runtime — what actually runs this on Metal from Rust

Four candidate paths. Only one survives.

**`stable-diffusion.cpp` + a Rust FFI — the recommended path.**
[leejet/stable-diffusion.cpp](https://github.com/leejet/stable-diffusion.cpp) is **MIT** [VENDOR], very
active (release `master-841`, 2026-08-30), GGUF-native, has a **C API**, and already supports SD1.x/2.x,
SDXL, SD3/3.5, FLUX.1, **FLUX.2, Z-Image, Qwen-Image, Chroma** — and, separately interesting, Wan 2.1/2.2,
LTX-2.x and HunyuanVideo 1.5. Metal is a listed backend. It is the same ggml family as the `llama-cpp-2`
we already link, so the build infrastructure exists.

Two caveats that are not optional reading:

- **The official macOS release binary is CPU-only.** `CMakeLists.txt:90` is
  `option(SD_METAL "sd: metal backend" OFF)`, and the macOS CI job builds without `-DSD_METAL=ON`
  [SOURCE, read from the repo]. We must build it ourselves with Metal on. Users reporting "Z-Image is slow
  on Mac" in issue #1030 are running the CPU binary without realising.
- **Metal is second-class and quant kernels break there first.** Open: #1847 (HiDream Q8 → pure white
  images on Metal, Aug 2026), #1330 ("Optimal use of Metal backend", Mar 2026, zero comments), #1145,
  #640. Recently closed: #1298 — `GGML_ASSERT(buft)` crash on the **second** `generate_image()` call
  reusing one `sd_ctx_t`. That last one is exactly our daemon shape: either confirm the fix or create a
  fresh context per generation.

Rust binding: [newfla/diffusion-rs](https://github.com/newfla/diffusion-rs), MIT, v0.1.20 (2026-06-16),
~10.7k downloads, 53 stars, one maintainer who states he cannot test on macOS. **[OURS] Expect to vendor
or fork the FFI shim.** That is still an order of magnitude cheaper than a Python sidecar.

**`candle` — attractive on paper, stale in practice. Do not lead with it.**
This is worth spelling out because the workspace already depends on it and it looks like the free option.
First-hand: `candle-transformers` **0.11.0 — the exact version in our `Cargo.lock` — ships
`models/flux/` (including `quantized_model.rs`), `models/mmdit/` (SD3), `models/stable_diffusion/`,
`clip`, `t5`, and `Config::schnell()`** [SOURCE, read from `~/.cargo/registry`]. The `metal` feature is
already enabled in `crates/goose/Cargo.toml:229`. So the capability is nominally in the tree.

But: candle's diffusion **examples** are frozen at 2024 (`examples/flux` last touched 2024-10-02,
`stable-diffusion-3` 2024-11-01), there is no FLUX.2 / Z-Image / Qwen-Image support, and
[candle #2406 — "Metal error with Flux: `cast_bf16_f32` does not exist"](https://github.com/huggingface/candle/issues/2406)
has been **open since August 2024**, with reporters confirming failure on M1 and M3. #2417 (SD img2img
null-ptr assert on Metal) is also open. **[OURS] "No new dependency" is illusory: choosing candle means
maintaining a 2024-era FLUX.1 implementation on a known-broken Metal path, for a model we've already
established is too big. Worth half a day to confirm the bug is dead; not worth planning around.**

**MLX / `mflux` — the quality ceiling, not a shipping path.**
[mflux](https://github.com/mflux-community/mflux) (MIT, pushed 2026-09-01) is the most capable
Apple-native option and supports every model on our list. It is **Python + `uv` + `mlx`**. Shipping it
means shipping a Python runtime through Developer ID signing and notarization. `mlx-rs` binds the array
framework only — there is **no diffusion model implemented in Rust for MLX**; we would be porting FLUX.2
ourselves. **[OURS] Use mflux on the M4 to measure what good looks like. Do not ship it.**

**ONNX / Core ML — dead end, despite `ort` and `libonnxruntime.dylib` already being in the bundle.**
`apple/ml-stable-diffusion` last committed **2025-07-03** and tops out at SD3-Medium/SDXL. DiffusionKit's
repo was **archived 2026-03-21**. No maintained ONNX diffusion path for modern DiT models on macOS was
found. **The 27 MB of ONNX Runtime we already ship buys us nothing here** — which is worth saying out
loud, because it is the intuitive first guess.

Two more that are real but cannot be the default:

- **Draw Things** — [drawthingsai/draw-things-community](https://github.com/drawthingsai/draw-things-community)
  is **GPL-v3**, exposes a `gRPCServerCLI` on port 7859 and an **A1111-compatible HTTP API**
  (`POST /sdapi/v1/txt2img`, `/img2img`, typically port 7860) [VENDOR, read from
  `HTTPAPIServer.swift`]. The README states client apps may use different licenses **provided they avoid
  bundling or automatic server downloads**. So: a great *optional* "use my local Draw Things" backend for
  power users; **legally unusable as a bundled default.** Their commercial `MediaGenerationKit` is
  quota'd (20 tasks/mo free) and key-gated — a per-user vendor dependency with a privacy story we don't
  control.
- **ComfyUI** — excellent REST+WebSocket API and official macOS/MPS support, but a GPL-3 Python app with
  a plugin ecosystem. Same "bring your own" category. Not shippable in a signed DMG.

And one to actively avoid: **Ollama cannot generate images right now.** Image generation shipped
experimentally 2026-01-20 (with, tellingly, exactly our two Apache-2.0 candidates), then was **removed in
v0.32.6**. As of issue #17893 (filed 2026-08-20, tested on v0.32.14) `/api/generate` returns
`"image generation models are not currently supported"` while `/api/tags` still advertises
`"capabilities": ["image"]`. No maintainer response. **Do not route the Librarian's Ollama through this.**

#### 1.1.3 Speed and memory — the honest numbers on 16 GB

All [MEASURED], each with machine and settings named.

| Machine | Stack / model | Result |
| --- | --- | --- |
| **M1 Pro 16 GB** | mflux, FLUX.1-schnell unquantized, 2 steps | **175.8 s**, "~22 GB swap … Running any other program at the same time will lock up and force reboot" (mflux #7) |
| **M4 Mac mini 16 GB** | mflux, schnell 4-bit, **512×512**, 2 steps | **7.60 s/it warm (~15 s total)**. **"1024×1024 not feasible due to memory constraints"** (mflux #105) |
| **M3 Pro 18 GB** | mflux, FLUX.2-klein 4-bit, 1024×1024 | **peak 14,032 MB — of which 9,629 MB is VAE decode alone**; weights only 4.4 GB. Author: "tight on 16 GB machines." VAE tiling at 512 → 7,178 MB (**−49%**), at 256 → 6,326 MB (−55%), **no speed or quality change**. **~15 s/step** (mflux #407; shipped as `--vae-tiling` in #475) |
| **MacBook Air M1 16 GB** | stable-diffusion.cpp **Metal**, Z-Image Q3 | **~20 s/iteration** — 10× slower than an RTX 1000 Ada 6 GB. 8 steps ≈ **160 s** (sd.cpp #1145) |
| M5 Max 128 GB (ceiling) | mflux, Z-Image-Turbo 768×768 | ~5 s @ 4 steps, ~9.5 s @ 8 steps (mflux #386) |

**[OURS] What this means for our two boxes.**

- **1024² on 16 GB is marginal and requires VAE tiling.** Without it, FLUX.2-klein-4B peaks at ~14 GB on
  an *18 GB* machine. With Tauri, `permagentd`, and macOS resident, 16 GB will swap. VAE tiling is
  non-negotiable, and it is free (no speed or quality cost, per the measurement above).
- **Expect 30–90 s per 1024² image on the M4/16 GB** at 4–8 steps with tiling, and **2–3× worse on the
  M1**, extrapolating from the M1 Air's 20 s/it.
- **The M1 headless mini is a poor fit.** Treat it as "512–768², slowly."
- **The exact cell we need is empty in the public record.** No published, verified end-to-end 1024²
  wall-clock exists for Z-Image or FLUX.2-klein on a **16 GB M4 via stable-diffusion.cpp with Metal on**.
  Every number above is a different chip, a different RAM size, a different runtime, or CPU-only.
  **This is the single highest-value experiment to run before any integration work — about half a day.**

#### 1.1.4 Quality floor — is local good enough to be the default?

Artificial Analysis text-to-image Elo, fetched 2026-09-01 [VENDOR]:

| Model | Elo | Open weights | Bundleable |
| --- | --- | --- | --- |
| **GPT Image 2 (high)** | **1370** | no | — |
| MAI-Image-2.6-Preview | 1349 | no | — |
| Nano Banana 2 (`gemini-3.1-flash-image`) | 1321 | no | — |
| Nano Banana Pro (`gemini-3-pro-image`) | 1296 | no | — |
| Qwen-Image-3.0-Pro | 1283 | no | — |
| Seedream 5.0 Pro | 1280 | no | — |
| FLUX.2 [max] | 1225 | no | — |
| **Ideogram 4.0** | 1219 | yes | **no — non-commercial** |
| FLUX.2 [pro] | 1208 | no | — |
| FLUX.2 [dev] | 1198 | yes | **no — NCL** |
| Recraft V4.1 | 1197 | no | — |
| Imagen 4 Ultra *(shut down 17 Aug 2026)* | 1189 | no | — |
| FLUX.2 [klein] 9B | 1140 | yes | **no — NCL** |
| **Z-Image-Turbo** | **1133** | yes | **YES (Apache-2.0)** |
| Qwen-Image | 1076 | yes | yes, but 20B |
| Midjourney v6 | 1077 | no | — |
| SD 3.5 Large | 1034 | yes | conditional |
| **FLUX.1 [schnell]** | **1000** | yes | yes, but 9.7 GB and slow |

FLUX.2-klein-4B is not separately rated; **[OURS] it will sit below the 9B's 1140**.

The bundleable ceiling is **Z-Image-Turbo at 1133 against GPT Image 2 at 1370 — a ~237 Elo gap**, and the
best *open* model we could ship is ~86 Elo behind the best open model we *can't* (Ideogram 4).

Where that gap actually shows up:

- **Text inside the image** — the sharpest divide. GPT Image 2 and Nano Banana 2 render paragraphs and
  UI mockups reliably. Z-Image-Turbo is the best open model at text (English *and* Chinese) and is
  decent, but not paragraph-reliable.
- **Compositional prompt adherence** ("three objects, specific spatial relations, specific attributes
  each") — this is where most of the 100–150 Elo lives.
- **Editing** — the open-weights editing leaderboard tops out at HunyuanImage 3.0 Instruct (1223),
  FLUX.2 [klein] 9B (1166), Qwen Image Edit Plus (1165). All are either non-commercial or far too big.
  **Bundleable local *editing* on 16 GB is essentially not a thing.**
- **Photorealism** — this is where the gap is *smallest*. Z-Image-Turbo is specifically strong on
  portraits and character photorealism.

**[OURS] Verdict on "default or fallback": a task-based split, which is now the ruling (§1.1.6).** A
bundled local model should not be the silent default for an agent asked to make a real deliverable, and
cloud must never touch a personal photo. Local owns drafts, thumbnails, iteration passes, offline and
privacy mode, and **everything involving the user's own images**. Cloud owns "make the final asset from a
text prompt" and anything with text rendered in it. Label the mode honestly in the UI — a user who puts one
Z-Image output next to one GPT Image 2 output will see the difference immediately, and the standing rule is
that we never claim local when it's cloud, nor imply parity we don't have.

### 1.1.5 Cheapest-viable survey — what else is out there, and why none of it wins

Jesse asked for the cheapest viable local option rather than a cloud spend threshold, and pointed at
[anil-matcha/open-generative-ai](https://github.com/anil-matcha/open-generative-ai). **[OURS] That list
adds nothing.** It catalogues 400+ models across 8 categories, but its open-weights *image* entries are
SD 1.5 finetunes (Dreamshaper 8, Realistic Vision v5.1, Anything v5, ~2.1 GB each), SDXL Base 1.0
(~6.9 GB), and Z-Image Turbo/Base — all already in §1.1. Everything else it lists (Flux pro, Nano Banana,
Midjourney, Ideogram, Seedream, GPT-4o) is a proprietary API. Its sibling lists are API price-comparison
tables, not model sources.

Searching independently past it, four families were worth checking. Only one was a genuine near-miss.

| Candidate | Params | License — **verified** | Working set | Verdict |
| --- | --- | --- | --- | --- |
| **NVIDIA Sana 0.6B / 1.6B** | 0.6B / 1.6B | **Apache-2.0** [VENDOR] — *plus* Gemma Terms of Use + Gemma Prohibited Use Policy, inherited from its encoder | ~3.1 GB (0.6B + Gemma-2-2B Q4 + DC-AE) | **The near-miss. Blocked on runtime — see below.** |
| **NVIDIA Sana-Sprint 0.6B / 1.6B** | 0.6B / 1.6B | Apache-2.0 [VENDOR] | ~3.1 GB | Same block. 1–4 step distilled; the most interesting of the family. |
| **SDXL-Turbo / SD-Turbo** | 3B / 0.9B | **`sai-nc-community` — Stability AI *Non-Commercial* Research Community License** [VENDOR, model card metadata] | 3.5–6.9 GB | **NO. Ambiguity resolved: non-commercial.** The §1.1 table flagged this as contradictory; it is now settled, and settled against us. |
| **PixArt-Sigma XL-2** | 0.6B | `openrail++` [VENDOR] | ~5.0 GB (T5-XXL encoder dominates) | No. Pass-through obligations *and* a 4.3 GB encoder — no smaller than klein-4B for worse output. |
| **Chroma1-Flash** | 9B | Apache-2.0 [VENDOR] | ~10 GB+ (FLUX.1 lineage, T5-XXL) | No. Clean licence, wrong size. |
| **Stable Cascade** | — | non-commercial | — | No. |

**Sana is the one that hurts, and the blocker is runtime, not licence or size.** It is genuinely clever —
a 32× spatially-compressed DC-AE latent (vs 8× for everything else) is *why* it is small and fast, and
NVIDIA pitches it as "deployable on laptop GPU". But **`stable-diffusion.cpp` does not support Sana**, and
neither does `candle`. Verified against the sd.cpp supported-model list [VENDOR]: SD1.x, SD2.x, SD-Turbo,
SDXL, SDXL-Turbo, SD3/3.5, FLUX.1, **FLUX.2-klein**, Z-Image, Qwen-Image, Chroma — **no Sana, no PixArt, no
Stable Cascade.** Running Sana means Python + `diffusers`, which is precisely the dependency this codebase
engineered *out* of the TTS path to get a "GPL-clean shipping backend". A ~1.9 GB saving does not buy a
Python runtime through Developer ID signing and notarization.

Two secondary marks against it even if the runtime existed: it is a 2024-era model (roughly SDXL-class,
well below Z-Image-Turbo), and its Apache-2.0 headline is qualified by **Gemma Terms of Use pass-through**
from the Gemma-2-2B encoder — the same shape of EULA obligation that made us decline OpenRAIL.

**So the survey's real yield is a cheaper *quantization*, not a cheaper model.** Exact GGUF byte sizes for
the DiT [VENDOR, HF blob listing]:

| FLUX.2-klein-4B quant | DiT bytes | + Qwen3-4B encoder | **Working set** |
| --- | --- | --- | --- |
| Q2_K | 1,827,807,808 (1.83 GB) | ~1.6 GB (Q2) | **~3.6 GB** ⚠️ Q2 on a diffusion transformer is usually visibly degraded — pilot before believing it |
| **Q3_K_S** | 2,101,928,512 (2.10 GB) | ~1.9 GB (Q3) | **~4.2 GB** ← **the new cheapest-viable floor** |
| Q4_K_S | 2,583,077,440 (2.58 GB) | ~2.29 GB (IQ4) | ~5.0 GB — the quality-safe default |
| Q8_0 | 4,300,644,928 (4.30 GB) | ~4.3 GB | ~8.8 GB — no headroom on 16 GB |

Note the klein GGUF repo ships **the DiT only** — encoder and VAE are sourced separately, so the working
set is always the sum of three downloads, and the encoder is roughly half of it. **[OURS] Ship Q3_K_S as
the floor and Q4_K_S as the default, and make the encoder quantization a separate knob** — dropping the
encoder from IQ4 to Q3 saves ~400 MB and text encoders degrade more gracefully than diffusion transformers.

**Nothing found fits the app bundle.** The smallest credible working set is ~3.6–4.2 GB. The only
sub-2 GB image model in existence with a redistributable licence is SD 1.5 at 2.14 GB (OpenRAIL-M,
2022 quality) — and it is *still* above 2 GB. **A bundled image model is not available at any acceptable
quality. First-run download is the only option, and Jesse has approved it.**

### 1.1.6 The ruling, and what it changes

Jesse's decisions, recorded here because they close three of the open questions and change the matrix:

1. **Personal photos never leave the machine. Cloud photo-editing is out of scope entirely.** Only
   text-prompt cloud generation ships. This is not a per-call confirmation — it is a capability that does
   not exist in the build.
2. **Task-based local/cloud split is the default**, not a global toggle.
3. **A ~5 GB first-use download is approved.**

**[OURS] The consequence is larger than it looks, and it is mostly good.** Removing cloud editing deletes
the single riskiest data path in the whole design — the one where a user's photograph is uploaded to a
third party — and with it most of §4.4's indemnity exposure, the mask-support comparison shopping
(FLUX.1 Fill vs Stability vs Google's absent masking), and the reference-image retention question against
BFL's 10-minute URLs. The cloud surface collapses to **one call shape: text prompt in, image out.**

It also makes §1.1.4's honest conclusion load-bearing: **local editing on 16 GB is essentially unavailable,
so "photos stay local" means image *editing* is not a shipping capability at all in v1.** That should be
stated plainly in the product rather than discovered by a user. Generation from a text prompt — local or
cloud — is what we ship.

The one place to watch: an agent asked to "clean up this screenshot" or "remove the background from this
photo" will reach for an edit tool. There must be no such tool wired to a cloud provider, and the local
one does not exist yet. **Non-generative alternatives cover more of this than expected** — Recraft's
remove-background is a cloud call and therefore out, but Apple's Vision framework does subject lifting and
`VNGenerateForegroundInstanceMaskRequest` entirely on-device, and Core Image handles crop/scale/filter
locally. Those are the right answers for "edit my photo", and they are not generative at all.

### 1.2 The cloud tier — escalation, priced

Prices are per image at 1024×1024 / 1K unless noted. All [VENDOR] from the provider's own pricing page
except where marked.

> **Scope note.** Per §1.1.6, **only text-prompt generation ships.** Editing, inpainting, mask and
> reference-image capabilities are documented below for completeness and for the day the ruling changes —
> they are **not** to be wired. When reading the comparison, ignore the editing columns: the selection
> criteria that actually apply are text-to-image quality, text-in-image fidelity, transparent background,
> and price.

#### 1.2.1 OpenAI — gpt-image line

Docs moved: `platform.openai.com/docs/*` now 301s to **`developers.openai.com/api/docs/*`**.

Models: `gpt-image-2` (`gpt-image-2-2026-04-21`, launched 21 Apr 2026, the first OpenAI image model that
**reasons before generating**), `gpt-image-1.5`, `gpt-image-1` (legacy), `gpt-image-1-mini`,
`chatgpt-image-latest`, plus legacy DALL·E.

Token rates, verbatim [VENDOR] ($/1M tokens):

| Model | Text in | Image in | Cached in | **Image out** |
| --- | --- | --- | --- | --- |
| gpt-image-2 | $5.00 | $8.00 | $2.00 | **$30.00** |
| gpt-image-1.5 | $5.00 | $8.00 | $2.00 | **$32.00** |
| gpt-image-1 | $5.00 | $10.00 | $2.50 | **$40.00** |
| gpt-image-1-mini | $2.00 | $2.50 | $0.25 | **$8.00** |

Batch (24 h, `/v1/batch`) is exactly half. Per-image at 1024²: OpenAI **no longer publishes a table** —
the pricing page defers to a client-side calculator. Third-party-derived [UNVERIFIED for gpt-image-2,
[DERIVED] and self-consistent for the rest]:

| Model | low | medium | high |
| --- | --- | --- | --- |
| gpt-image-2 | ~$0.005–0.006 | **$0.053** | **$0.211** |
| gpt-image-1.5 | $0.009 | $0.034 | $0.133 |
| gpt-image-1 | $0.011 | $0.042 | $0.167 |
| gpt-image-1-mini | $0.005 | $0.011 | $0.036 |

gpt-image-2 widescreen/portrait is *cheaper* than square: $0.041 medium / $0.165 high. The 1.5/1/mini rows
reconcile exactly with the documented 272 / 1056 / 4160 output-token counts at 1024²; the gpt-image-2 rows
do not fit that schedule, so treat them as good-but-secondary.

API shape: **synchronous** `POST /v1/images/generations` and `/v1/images/edits`, *plus* the
`image_generation` built-in tool in the Responses API. Uniquely: **streaming partial images**
(`stream: true`, `partial_images` 0–3 — each partial costs **+100 image output tokens**, so it is a paid
latency-perception feature). Edits accept **up to 16 input images** plus a `mask` for true inpainting.
`background: "transparent"` (PNG/WebP only) — GA on 1/1.5, **preview on gpt-image-2**. Output is
**base64 only**, no hosted URL, so there is no expiry to race. Moderation blocks return a structured
`moderation_stage` (input/output) and `categories`.

Rate limits [VENDOR]: **IPM (images/min) + TPM**, identical across mini/1.5/2 — Tier 1: 5 IPM / 100K TPM ·
T2: 20 / 250K · T3: 50 / 800K · T4: 150 / 3M · T5: 250 / 8M. Tiers unlock at $5 / $50 / $100 / $250 /
$1,000 cumulative paid.

Latency: **P50 32.18 s end-to-end** for gpt-image-2 [MEASURED, OpenRouter]. The guide says complex prompts
"may take up to 2 minutes."

Licensing: API data not trained on by default; abuse logs ≤30 days; ZDR for approved customers [VENDOR].
Output ownership ("you own Output") is in the Terms of Use — **[UNVERIFIED this session:
`openai.com/policies/*` returns 403 to fetchers.** Whether gpt-image-2 API bytes carry a C2PA manifest is
also **[UNVERIFIED]** — the image-generation guide never mentions C2PA. Inspect returned bytes for a JUMBF
box before relying on either answer.

#### 1.2.2 Google — Gemini native image. **Imagen is gone.**

**Imagen shut down 17 Aug 2026** (deprecation announced 15 Jun 2026) [VENDOR, changelog]. It no longer
appears on the pricing page at all. Any plan that budgeted on Imagen 4 is obsolete.

| Marketing name | Model ID | 1K price | Notes |
| --- | --- | --- | --- |
| Nano Banana Pro | `gemini-3-pro-image` | **$0.134** (1K & 2K), $0.24 @4K | up to 6 reference images, interleaved text+image output |
| Nano Banana 2 | `gemini-3.1-flash-image` | **$0.067** ($0.045 @0.5K, $0.101 @2K, $0.151 @4K) | **up to 14 reference images** (10 object + 4 character), video-to-image, Search grounding |
| Nano Banana 2 Lite | `gemini-3.1-flash-lite-image` | **$0.0336** | 1K only, no multi-image composition |
| Nano Banana (legacy) | `gemini-2.5-flash-image` | $0.039 flat | migrate off |

Batch is exactly half throughout. **No free tier on any image model** [VENDOR].

API shape changed: image generation now runs through the **Interactions API**
(`POST /v1beta/interactions`) with `response_format: {type: "image", mime_type, aspect_ratio, image_size}`
where `image_size` ∈ `512px | 1K | 2K | 4K` (**uppercase K required**). Conversational editing uses
`previous_interaction_id` plus **thought signatures**; interim "thought images" are visible in response
steps and **not charged**. Output is inline base64.

**Two hard limitations.** There is **no mask-based inpainting** — editing is semantic only. And
**transparent background is not supported or documented**. If our use case needs masked edits or alpha,
Google is out.

**Every generated image carries a SynthID watermark**, stated flatly with no developer opt-out [VENDOR].
Google's stack makes no mention of C2PA.

Licensing [VENDOR]: "Google won't claim ownership over that content… Google may generate the same or
similar content for others." Paid tier is not trained on; **free tier is, and "human reviewers may read,
annotate, and process your API input and output."** That alone means the free tier must never see a user's
private material.

Rate limits: Google no longer publishes per-model numbers — AI Studio dashboard only [VENDOR]. Metering is
in IPM. Spend limits per rolling 10 min: T1 $10, T2 $50, T3 $200.

#### 1.2.3 Black Forest Labs — FLUX

**FLUX 3 is the video line.** For images, "FLUX.2 remains fully supported for production image generation
and editing" [VENDOR].

| Model | Text-to-image | Editing |
| --- | --- | --- |
| FLUX.2 [klein] 4B | $0.014 | $0.014 |
| FLUX.2 [klein] 9B | $0.015 | $0.015 |
| **FLUX.2 [pro]** — recommended default | **$0.03** | $0.045 |
| **FLUX.2 [flex]** — *typography and small-detail specialist* | $0.05 | $0.05 |
| FLUX.2 [max] — highest quality + editing consistency | $0.07 | $0.07 |
| FLUX.1 Kontext [pro] / [max] | $0.04 / $0.08 | ″ |
| FLUX.1 Fill [pro] — **the only masked-inpaint endpoint** | $0.05 | — |

Megapixel-based for FLUX.2 ("from" = 1 MP); 1 credit = $0.01 [VENDOR].

API shape: **fully asynchronous.** POST → `{id, polling_url, cost, input_mp, output_mp}` → poll, or use
`webhook_url` + `webhook_secret`. **Signed result URLs are valid for 10 minutes only** — this is the
tightest retention window of any provider and it dictates architecture (§4.5). Concurrency: **24 active
tasks** (6 for `flux-kontext-max`), 429 over that, 402 on depleted credits.

Up to **8 reference images** on flux-2-pro/flex. **FLUX.2 dropped the `mask` parameter that FLUX.1 Fill
has, and has no `aspect_ratio` (width/height only)** — so BFL's masked-edit path is stuck on the older,
lower-quality model. No transparent background anywhere.

Licensing [VENDOR]: "**As between you and us, you own all right, title, and interest in and to Output**",
commercial use permitted. Prohibited: using outputs to train/distill/fine-tune other models, or to build a
competing product. BFL "reserve[s] the right to **embed Content Credentials or other provenance data in
any Output**… without prior notice" — **assume C2PA may be present**, and removing AI content marking is
forbidden.

#### 1.2.4 The rest, briefly

- **Ideogram 4.0** — the typography specialist, and the only one with third-party *design-professional*
  evaluation: ContraLabs had designers pick Ideogram 4 best **47.9%** of the time vs **30.0%** for Nano
  Banana 2 [VENDOR-cited third party]. $0.03 Turbo / $0.06 Default / $0.10 Quality. **Dedicated
  transparent-background endpoints** at the same price at 1K/2K (but $0.19–$0.42 at 4K/8K). Sync *and*
  async with Ed25519-signed webhooks. **Awkward split: 4.0 has no mask endpoint and no style/character
  reference — those live on 3.0.** Rate limit is a **10-inflight concurrency cap**, ~2 orders of
  magnitude tighter than Stability; any batch work needs a queue in front. P-Image tier goes down to
  **$0.003/image** and publishes a **2.9–5.0 s** latency range — the only provider that publishes latency
  at all.
- **Recraft V4.1** — $0.035 raster, **$0.08 vector (true SVG)**, remove-background $0.01, vectorize $0.01.
  **[OURS] The right answer when the asset must be editable and brand-controlled rather than merely look
  right** — an SVG we can restyle in code beats a raster we can't.
- **Stability** — no new model line in 2026; SD 3.5 family + Stable Image Ultra/Core. $0.025 (3.5 Flash)
  to $0.08 (Ultra). Mostly synchronous. Full mask inpainting and a rich edit-op catalogue
  (erase/inpaint/outpaint/remove-background/search-and-replace, 4–8 credits each). Loosest rate limit
  here: 429 only past **150 requests / 10 s**. Community License with the same **$1M revenue cliff**.
- **xAI Grok Imagine** — cheapest credible: `grok-imagine-image` **$0.02**, 2.0 $0.04, quality $0.05.
  Synchronous, up to 10 images/request, editing with 5 source images. No transparency, text not a strength.
- **ByteDance Seedream** — `seedream-4-0-250828` at **$0.03**, charged only on success, **500 images/min**,
  sync and async. Seedream 4.5 scored **4.93/5.00** on text-heavy prompts, ahead of GPT Image 1.5 (4.88)
  and FLUX.2 Pro (4.83) [third-party eval]. 5.0 Pro is flagship (~$0.0675 via fal).
- **Qwen-Image** — cheapest at ~$0.025–0.07, best CJK, but degrades sharply (0.41) on **mixed CJK+Latin in
  one image** where Seedream 4.5 holds at 0.82.
- **Luma** — **Photon no longer exists as a separate product**; current model is **UNI-1.1** (5 May 2026),
  $0.0404/image at 2048 px, ~31 s. If anything budgeted on Photon Flash's old $0.0019, that is obsolete.
- **Reve** — real API, but publishes **no price table**; docs tell you to inspect `credits_used`.
  Committed prices exist only via fal ($0.04).
- **Midjourney — still no official API as of Sept 2026**, and its ToS explicitly prohibits automated
  access. Every "Midjourney API" on sale is an unofficial Discord-account proxy. **Do not put it in a
  production path.**

#### 1.2.5 The two rankings that matter for our use cases

**Transparent background, natively:** OpenAI (`background: "transparent"`, free, no extra call) →
Ideogram (dedicated endpoints, same price at 1K/2K) → Recraft (via true SVG, or +$0.01 remove-background)
→ Stability (post-hoc, $0.05). **Google, BFL, xAI, Luma, Reve, Seedream and Qwen have none.**

**Text rendering in images:** gpt-image-2 (the reasoning pass is the differentiator; also the only one
that renders convincing UI screenshots) → Ideogram 4.0 → Seedream 5.0 Pro / 4.5 → FLUX.2 [flex] →
Nano Banana Pro → Recraft V4.1 (when the text must stay editable) → Qwen (CJK only). Weakest: Stability,
Luma, Grok.

### 1.3 Apple's own — a dead end, and Apple says so

This one has a clean answer and it is worth stating plainly because it is counter-intuitive given our
standing preference for on-device.

**`ImageCreator`, the non-UI programmatic image API, is deprecated.** Apple's developer news post of
**11 June 2026** [VENDOR, quoted]: "the ImageCreator class is being discontinued and will **no longer work
in iOS 27, iPadOS 27, macOS 27, and visionOS 27** or later." In beta OS releases "your code will continue
to compile, but you'll begin to receive warnings… Apps using ImageCreator **will not function in TestFlight
builds** and will cause a runtime error." In public releases, "your code won't compile."

Its replacement is **UI-only**. WWDC26 session 375: "ImageCreator, the non-UI API for generating images
directly in your code, is deprecated. Everything is now available through a new API… built on a full
experience people already know how to use." The surface is `imagePlaygroundSheet(...)` in SwiftUI and
`ImagePlaygroundViewController` in AppKit — **a modal sheet the user drives**. There is no headless call.

And the second surprise: **it is no longer on-device.** The 2026 Image Playground model "runs on **Private
Cloud Compute**" [VENDOR, WWDC26]. Quality went up — photorealism, people, multiple aspect ratios via
`sizeSpecification = .closest(to:)` — but it left the device. Usage limits are system-managed and scale
with iCloud+ tier. Styles: illustration, sketch, animation, Genmoji, free-text style descriptions, or an
external provider (ChatGPT) if the user configured one in Settings.

**[OURS] Three consequences.**

1. **Apple offers us nothing usable.** An agent that generates an image as a step in a longer task cannot
   stop and present a modal sheet. Apple's own migration note even says: "**Alternatively, you can
   integrate another image generation service of your choice.**" That is Apple telling developers to go
   elsewhere for programmatic generation.
2. **It would not satisfy the privacy preference anyway.** PCC is a strong privacy story, but it is
   still off-device, and our standing rule is that we never present cloud as local. Image Playground
   would have to be labelled as cloud in our UI, at which point it has no advantage over a provider we
   actually control.
3. **There is one narrow place it could still earn its keep** — a user-initiated "make me an image"
   affordance in the UI where a native sheet is the *right* interaction and costs us nothing per image.
   That is a product decision, not an agent capability. Genmoji (`NSAdaptiveImageGlyph` via
   `onAdaptiveImageGlyphCreation`) is the same shape.

`@Environment(\.supportsImageGeneration)` is the availability check if we ever do wire the sheet.
---

## 2. Video generation

**Short version: stub it. Define the abstraction, ship no provider yet.** The reasoning below is worth
reading anyway, because two of the findings would silently break a plan written last month.

### 2.1 Three findings that change the shape of the decision

**1. The OpenAI Sora API dies on 24 September 2026 — 23 days from today — with no replacement.**
Verbatim from OpenAI's deprecations page [VENDOR]: "On March 24th, 2026, we notified developers using the
Videos API and Sora 2 video generation model aliases and snapshots of their deprecation and removal from
the API on **September 24, 2026**." The "Recommended replacement" column reads `---`. No `sora-3` has been
announced, and the consumer Sora app shut down 26 April 2026. **Anything that names Sora is already dead
code.**

**2. Google's video default is no longer Veo.** It is `gemini-omni-1.1-flash`, GA since **27 Aug 2026**,
token-billed, 40 s max, native audio, conversational multi-turn editing via `previous_interaction_id`
[VENDOR, changelog]. Veo 3.1 is now explicitly positioned as the fallback — the video overview page says
"Use Veo 3.1 for specific capabilities like scene extension, last-frame control, or integration with
**legacy pipelines**." And `veo-3.0-*` / `veo-2.0-*` were **shut down on the Gemini API on 30 June 2026**.

**3. A 16 GB Apple Silicon Mac cannot generate video.** Not "slowly" — see §2.4.

### 2.2 Cloud pricing, per second of output

$/s at the cheapest configuration including audio where audio exists; ✱ = no audio at any price.
All [VENDOR] unless noted.

| Provider | Model | **$/s** | Max duration | Audio | API status |
| --- | --- | --- | --- | --- | --- |
| fal | `minimax/h3-max` 768p | **$0.02** promo → $0.08 | 15 s | yes | GA (promo ends 7 Sep) |
| Pika | Pika 2.5 720p / 1080p | $0.04 / $0.09 | 10 s | — | GA, self-serve |
| **Kling** | 2.5 Turbo 720p | **$0.042** ✱ | 10 s | no | GA |
| Google | **Veo 3.1 Lite 720p** | **$0.05** ($0.03 on Vertex, video-only) | 8 s | yes | Preview |
| Runway | `gen4_turbo` | $0.05 | — | — | GA, self-serve |
| **xAI** | Grok Imagine Video 720p | **$0.050** | 15 s | yes | GA, self-serve |
| Replicate / fal | `alibaba/wan-3` 480/720/1080p | $0.05 / $0.10 / $0.20 | 30 s | yes | GA |
| fal | `minimax/h3` 768p | $0.06 | 15 s | yes | GA |
| Luma | `ray-3.2` 720p | $0.060 ✱ | 10 s | **no** | GA |
| dev.pika.art | Kling 3.0 1080p | **$0.09** | 15 s | — | GA |
| **Google** | **`gemini-omni-1.1-flash` 720p** | **$0.101** | **40 s** | yes | **GA** |
| Google | Veo 3.1 Fast 720p | $0.10 | 8 s | yes | Preview |
| ~~OpenAI~~ | ~~`sora-2` 720p~~ | ~~$0.10 ($0.05 batch)~~ | ~~20 s~~ | yes | **removed 24 Sep 2026** |
| Runway | `gen4.5` | $0.12 | — | yes | GA |
| Google | Gemini Omni 1080p | $0.152 | 40 s | yes | GA |
| **Kling** | 3.0 1080p + audio | $0.168 | **15 s** | yes | GA, self-serve internationally |
| Luma | `ray-3.2` 1080p | $0.240 ✱ | 10 s | no | GA |
| Runway | `aleph2` (video-to-video) | $0.28 | — | — | GA |
| Moonvalley | Marey (fal only; first-party is waitlist) | $0.30 | 10 s | — | GA via fal |
| Google | Veo 3.1 1080p / 4K | $0.40 / $0.60 | 8 s (+7 s ×20) | yes | Preview |
| Kling | 3.0 4K | $0.42 | 15 s | yes | GA |
| ~~OpenAI~~ | ~~`sora-2-pro` 1080p~~ | ~~$0.70~~ | ~~20 s~~ | yes | **removed** |
| Replicate | open models (LTX, Hunyuan, Mochi, Wan 1.3B) | **per GPU-second** — H100 $0.001525/s | — | varies | GA |
| **Local, 16 GB Apple Silicon** | — | — | — | — | **not feasible** |

Google's Omni billing is token-based; the conversion is 1,931 tokens/s at 360p, 5,792 at 720p, 8,688 at
1080p, 17,376 at 4K, against $17.50/1M video output tokens [VENDOR]. Google charges nothing for a failed
render. Adobe Firefly Video **has no API endpoint at all** — every `generate_video` path in Firefly
Services 404s.

**Aggregators are at parity or below first-party on flagships** — nobody was found charging *above*
first-party on a flagship. The markup hides in the cheap/fast tiers (fal's Veo 3.1 Fast at $0.15 vs
Google's $0.10, +50%). For the Chinese labs, first-party English pricing is effectively unobtainable, so
**the aggregator rate is the market price**. `dev.pika.art` publishes the only **machine-readable,
unauthenticated price catalog** anywhere (`https://api.dev.pika.art/catalog/apis`) and undercuts both fal
and Replicate on several models.

### 2.3 What a clip actually costs, and the latency it costs

**[DERIVED] A single 10-second 1080p clip:** Veo 3.1 $4.00 · Kling 3.0 $1.68 · Gemini Omni $1.52 ·
Luma $3.60 (10 s costs **3×** the 5 s price, not 2×) · Runway gen4.5 $1.20 · Pika 2.5 $0.90.
The retiring Sora 2 Pro at 1080p was $7.00.

Set against Permagent's existing budget ceilings — **task $2 / $5 / $10, session $10 / $25 / $50**
[SOURCE, `cost_router/budget.rs`] — **one 10-second 1080p clip consumes between a fifth and four-fifths of
an entire task's hard ceiling.** That is the number that decides this section.

**Latency:** almost nobody publishes it. Luma is the honourable exception: 5 s/720p "well under two
minutes"; 10 s/1080p HDR "several times longer" [VENDOR]. fal's Hailuo-02 page says ~4 minutes. OpenAI
said "a single render may take several minutes." Everything else is [UNVERIFIED] third-party. **Plan for
1–5 minutes wall clock.** Every provider is async job + poll; only OpenAI, Kling and Replicate have real
webhooks (Runway's are undocumented; Luma and Google have none — Luma's FAQ says poll every 2–3 s).

### 2.4 Local video on 16 GB — a clear no

**Nothing is validated on 16 GB by an independent party. Zero reports with numbers**, across every repo,
issue tracker and blog checked.

The structural reason is unified memory: there is no second pool. On NVIDIA an 8 GB card runs Wan 2.2
TI2V-5B FP8 by offloading to 24–32 GB of *system* RAM. A 16 GB Mac must hold transformer + text encoder
(T5-XXL alone is ~9.6 GB bf16) + VAE + activations + macOS in one budget, with a GPU-addressable ceiling
around 12 GB.

Measured reality [MEASURED, each on a machine with 2–8× our memory]:

| Machine | Model | Result |
| --- | --- | --- |
| M4 Pro Mac mini **24 GB** | MiniMax-H3 4-bit, 832×480 | **28.5 min** — the smallest machine with any real number |
| M1 Max 64 GB | Wan 2.2 14B Q4, 832×480, 33 frames (~2 s of video) | **82 minutes** |
| M1 Max 64 GB | LTX-2 GGUF, 512×288, 33 frames | 6 m 54 s → *"colored blobs bouncing up and down"* |
| M4 Pro Mac mini 64 GB | CogVideoX-5B, 512×384, 30 frames | ~18 min; 49 frames ≈ 40 min at a 50% failure rate |
| M5 Max 128 GB | LTX-2.5 Fast, 540p / 5 s | 2 m 28 s, **43.14 GiB peak** |
| M3 Ultra 256 GB | MiniMax-H3 4-bit, 15 s @1344×768 | **426 minutes (7.1 hours)** |
| M4 Max (any RAM) | Wan 2.1 T2V-**1.3B**, 832×480, 48 frames | **~100 GB of RAM consumed** — and 1.3B is the smallest open video model in existence |

MLX video generation does now exist — `Blaizzy/mlx-video` (LTX-2, Wan 2.1/2.2), `lpalbou/mlx-gen` — but
it does not make anything small: mlx-gen on an **M5 Max**, at a deliberately tiny 384×224 / 33 frames /
12 steps, peaked at **27.7 GiB bf16 / 15.5 GiB q8**. Even quantized, at postage-stamp resolution, that
exceeds a 16 GB Mac's entire GPU budget.

Worth knowing but not worth planning on: `yhakami/dit-flash` applies the "LLM in a Flash" trick to video
DiTs, streaming transformer blocks from disk one at a time and claiming resident DiT memory drops from
~54 GB to ~0.7 GB, with Wan 2.2 A14B producing "a 5-second 480p clip in tens of minutes" on a 16 GB Mac
mini. **[UNVERIFIED] — one commit, one author, zero issues, no external replication, and "expect" is
projection language, not measurement.** The unpriced cost is streaming tens of GB per denoising step off
the slowest SSD in the Mac lineup.

**Licensing, for completeness**, because it would matter if the hardware ever caught up:
**Wan 2.2 TI2V-5B is Apache-2.0** [VENDOR] — 5B, 720p24, 5 s, both text-to-video and image-to-video,
GGUF quants from 1.85 GB (Q2_K) to 5.4 GB (Q8_0). It is the only genuinely bundleable-licence video model.
But its own card says "**at least 24 GB VRAM (e.g. RTX 4090)**" and "under 9 minutes" for 5 s of 720p *on
that 4090*. Extrapolating the ~10× Metal-vs-NVIDIA gap measured for images (§1.1.3), that is **an hour and
a half per five-second clip on our hardware, if it fits at all.** LTX-2.3/2.5 (19–22B) and MiniMax-H3
(33B) are open-weights but revenue-capped ($10M and $20M respectively), not Apache — and far too big
regardless. Wan **stops at 2.2**: 2.5 / 2.7 / 3.0 are closed API models with no published weights.

Also relevant and slightly surprising: `stable-diffusion.cpp` — our recommended *image* runtime — already
lists Wan 2.1/2.2, LTX-2.3/2.5, MiniMax-H3 and HunyuanVideo 1.5 among its supported models [VENDOR]. So if
the hardware situation ever changes, the runtime is already the one we would have chosen. That is an
argument for the abstraction, not for shipping video now.

**Verdict: 32 GB to attempt, 36–48 GB to validate against, 64 GB to work.** A 16 GB Mac mini's ceiling is
LLMs and image generation.

### 2.5 Recommendation: stub it, behind the same abstraction

**[OURS] Do not wire a video provider now.** Four reasons, in order of weight:

1. **The economics collide head-on with our own spend governance.** One 10-second 1080p clip is
   $1.20–$4.00 against a task hard ceiling of $10. Video is not a feature that fits inside the existing
   budget bands; it is a feature that requires its own consent flow every single time.
2. **The market is mid-reshuffle.** Sora's API dies in 23 days. Google replaced its own default a week
   ago. Veo 3.0 and 2.0 were shut off in June. Kling's entire legacy tier retires on 15 September. Luma
   re-platformed and dropped audio entirely. **Anything integrated today is likely to need rewriting
   within two quarters**, and none of that churn is our fault or under our control.
3. **There is no local story at all**, so video can never satisfy the privacy preference. Every clip is an
   egress.
4. **No use case has been named.** Images have obvious ones (illustrating a document, generating an
   asset, a diagram). Nobody has said what Permagent would generate video *for*.

**What to build instead:** define the media-generation abstraction (§4.1) with a `MediaKind` that admits
`Image` and `Video` from day one, and a provider trait whose cost estimate is a function of
`(kind, model, params)` rather than tokens. Register **no** video provider. When a use case appears, the
first integration should be **`gemini-omni-1.1-flash`** ($0.101/s, GA, 40 s, native audio, conversational
editing, SynthID, charges nothing for failures) or **fal** as a single key across Wan / Seedance / H3 /
Kling / LTX at parity-or-better pricing — and it should be a day's work, not a redesign.
---

## 3. Document production

This is the section where the bundling directive has the cleanest answer, and where we are closest to
already having built something — in two incompatible directions (`docx-rs` writes Word; `autovisualiser`
renders HTML charts).

### 3.1 Typst — the recommendation, and why

**Version and licence.** Typst **0.15.1**, released **2026-07-17** [VENDOR, crates.io]. Cadence is roughly
two minors a year plus patches; MSRV 1.92. The compiler is **Apache-2.0** — verified on
`typst.app/open-source` ("you are free to use it in commercial projects… and **embed it into other free or
commercial software**"), in the workspace `Cargo.toml`, and on crates.io for `typst`, `typst-pdf`,
`typst-html`, `typst-svg`, `typst-render`, `typst-library`, `typst-kit`. No CLA, no dual licence, no
commercial carve-out. The only obligation is shipping the `NOTICE` file. *(The Typst **web app** is
proprietary; that is a separate product and irrelevant to us.)*

**It is still 0.x.** Breaking language and API changes between minors are normal. **[OURS] Pin the version
and own the templates.** This is the single biggest non-technical risk in the whole Typst story: we would
be shipping a compiler whose language is still moving.

#### 3.1.1 Embedding — the real API surface

```rust
pub fn compile<T>(world: &dyn World) -> Warned<SourceResult<T>>  // T = PagedDocument | HtmlDocument
```

`World` has **exactly seven required methods** [VENDOR, docs.rs]:

```rust
fn library(&self)  -> &LazyHash<Library>;
fn book(&self)     -> &LazyHash<FontBook>;
fn main(&self)     -> FileId;
fn source(&self, id: FileId) -> Result<Source, FileError>;
fn file(&self, id: FileId)   -> Result<Bytes, FileError>;
fn font(&self, index: usize) -> Option<Font>;
fn today(&self, offset: Option<Duration>) -> Option<Datetime>;
```

Then `typst_pdf::pdf(&doc, &PdfOptions::default()) -> Warned<SourceResult<Vec<u8>>>`.

**It is not as small as it looks.** Typst's own reference implementation, `typst-cli/src/world.rs`, is
**366 lines** — virtual filesystem, `FileId`→path resolution across package roots, font book construction,
package downloader, and `comemo` cache invalidation, because `World` implementations are *required* to
cache (repeated calls with the same argument must return the same value or incremental compilation
misbehaves).

Three routes:

1. **`typst-as-lib` 0.16.0 (MIT, 2026-06-29, 863k downloads)** — wraps `World` in a builder.
   `TypstEngine::builder().main_file(T).fonts([F]).build()` → `compile_with_input(data)`. The author's own
   README says "**This API is currently not really stable.**"
2. **`tinymist-world` 0.15.0** — the LSP's production `World`. Heavier, battle-tested.
3. **Write our own `World` against `typst-kit`** (Apache-2.0). **[OURS] This is what a daemon that needs
   precise control over sandboxing and file access should do** — and it is the option that lets us serve
   packages and fonts entirely from the app bundle.

Real pain points people hit: math renders as **question marks** if the math fonts aren't shipped
(`typst-as-lib` #43, macOS); "no font could be found even though font book is populated" when embedding via
`include_dir!` (#54); and a build-time offline package-bundling PR (#34) was **closed unmerged** — offline
vendoring is our job. One blunt practitioner verdict on HN: *"Typst, which is nice externally but honestly
very difficult to use as a library."* Against that: **143 crates depend on `typst`**, including real
production users, so it is done in anger.

#### 3.1.2 Size and speed — measured

The official `typst-aarch64-apple-darwin` 0.15.1 binary is **45,029,488 bytes = 43 MB** uncompressed
(13.76 MB as `.tar.xz`) [MEASURED]. That includes embedded fonts (~9.5 MB), ICU data, clap, the watch
server and self-update; a library-only embed drops some of that but keeps fonts and ICU.
**[OURS] Budget 25–40 MB added to `permagentd`** — against a binary that is already 329 MB, that is a
~10% increase and no new runtime.

Compile time for a ~10-page US-Letter report with headers/footers, numbered headings, a **180-row
multi-page table**, a display equation and ~700 words: **70–90 ms steady state** (0.70 s first run,
page-cache cold) on Apple Silicon [MEASURED]. **That is fast enough to run synchronously in a request
handler and to do compile→fix→recompile agent loops at interactive speed** — which turns out to be the
thing that rescues Typst's weak LLM priors (§3.5).

#### 3.1.3 Fonts — 9.49 MB, all redistributable

`typst-assets` (Apache-2.0) bundles **31 font files, 9,947,809 bytes = 9.49 MB** [MEASURED]:

| Family | Files | Bytes | Licence |
| --- | --- | --- | --- |
| Libertinus Serif (default text) | 6 | 1.86 MB | **SIL OFL 1.1** |
| New Computer Modern | 4 | 2.68 MB | GUST Font License v1.0 (NewCM10-Regular is OFL) |
| NewCMMath (default math) | 3 | 3.97 MB | GUST Font License v1.0 |
| DejaVu Sans Mono (default `raw`) | 4 | 1.18 MB | Bitstream Vera / Arev permissive |
| Foxit base-14 PDF substitutes | 14 | 0.27 MB | permissive |

**All redistributable in a commercial DMG.** OFL's only real constraint is that you can't sell the fonts by
themselves and can't rename-and-reuse the reserved names. Plus 9.3 KB of ICC profiles (CC0) and 30 KB of
ICU CJK segmentation data.

Access is via `typst-kit::fonts`: `embedded()`, `system()` (via `fontdb`), `scan()`.
**[OURS] Ship the embedded set and disable system font scanning.** Deterministic output across machines
matters more than font variety, and system-font drift is exactly how "it looked fine on my Mac" bugs
happen. Add Inter and Manrope as bytes so documents match the UI type stack (`tokens.ts:35`) — both OFL,
~1 MB combined. **Do not skip the math fonts**; issue #43 is what happens when you do.

#### 3.1.4 Packages offline — tested, and it works

Typst Universe holds **1,561 packages / 4,647 versions** as of today; licence mix is **MIT 3,434 ·
MIT-0 350 · Apache-2.0 199 · Unlicense 124 · GPL-3.0-* 168 · LGPL-3.0 39 · AGPL 32**. Overwhelmingly
permissive — but **check per package** (see the CeTZ trap below).

The compiler resolves `@preview/name:version` from `{cache}/preview/{name}/{version}/`, overridable via
`TYPST_PACKAGE_CACHE_PATH` (and `TYPST_PACKAGE_PATH` for `@local`). Tested directly [MEASURED]:

```
# vendor dir pre-populated with lilaq 0.6.0 + its 3 transitive deps (1.1 MB total)
HTTPS_PROXY=http://127.0.0.1:9 TYPST_PACKAGE_CACHE_PATH=$VENDOR typst compile chart.typ
→ chart.pdf produced, zero network                                        ✅

# same, but importing a package NOT in the vendor dir
→ error: failed to download package (…Connection refused (os error 61))   ✅ fails loudly
```

Two gotchas: **transitive dependencies are not obvious** (vendoring `lilaq` alone was not enough — it
pulled `elembic`, `zero`, `tiptoe`), and there is **no `typst vendor` command** — you compile once online
and copy the cache. Tarballs are tiny (lilaq 101 KB, touying 109 KB, cetz 214 KB, polylux 6 KB), so a
curated vendor set is **~1–3 MB**.

**An embedded Typst compiler needs zero network access, provided we vendor the closure at build time.**
And because we implement `World` ourselves, package resolution is entirely under our control — we can
refuse network by construction rather than by environment variable.

#### 3.1.5 Features that matter for reports

- **Templating** — full scripting: `#let`, closures, `set`/`show` rules, `context`, and
  `json()`/`csv()`/`yaml()`/`toml()` data loading. `compile_with_input()` injects a Rust value as
  `sys.inputs`. **This is the highest-leverage pattern in the whole section (§3.5).**
- **Tables** — `table()` with `columns`, per-column `align`, `table.header` that **repeats across page
  breaks**, `colspan`/`rowspan`, strokes, fills. The 180-row test table paginated correctly with a
  repeating header [MEASURED].
- **Charts** — **`lilaq` 0.6.0, MIT**, matplotlib-shaped scientific plotting; worked offline, ~12 KB PDF
  [MEASURED]. **This is the one to ship.** ⚠️ **Avoid `cetz` 0.5.2 / `cetz-plot` — LGPL-3.0-or-later.**
  LGPL on a *source* package compiled into your document is legally odd; shipping it verbatim satisfies
  source conveyance, but there is no reason to have the argument. Note `fletcher` (MIT, node/arrow
  diagrams) depends on cetz, so it inherits the problem.
- **Page control** — `#set page(paper:, margin:, header:, footer:, columns:)`, `#pagebreak()`, `#place()`,
  `#counter(page)`. First-class. *(Compare §3.3: no browser engine can do running headers or page numbers
  at all.)*
- **Images** — PNG / JPEG / GIF / **SVG** / **PDF**, from path or `Bytes`. **[OURS] This is the bridge to
  our existing charts**: the `autovisualiser` HTML templates can render to SVG and be embedded directly,
  so we are not forced to reimplement eight chart types in lilaq on day one.
- **Math** — native with NewCMMath. **Bibliography** — Hayagriva + full CSL; 0.15 added native multiple
  bibliographies with automatic citation routing.

#### 3.1.6 PDF/A and tagged PDF — best in class, verified

Typst supports PDF **1.4 / 1.5 / 1.6 / 1.7 (default) / 2.0** and standards
**`a-1b, a-1a, a-2b, a-2u, a-2a, a-3b, a-3u, a-3a, a-4, a-4f, a-4e, ua-1`**, combinable. **Tagged PDF is on
by default** — "Typst will always write *Tagged PDF* to provide a baseline level of accessibility."

Verified [MEASURED]: a plain compile produced a PDF containing `/StructTreeRoot` and `/Marked`.
`--pdf-standard a-2a,ua-1` first failed with:

```
error: PDF/A-2a, PDF/UA-1 error: missing alt text
  = hint: make sure your images and equations have alt text
error: PDF/UA-1 error: missing document title
  = hint: set the title with `set document(title: [...])`
```

After adding `#set document(title:)` and `#math.equation(alt: "...")` it produced a valid PDF/A-2a +
PDF/UA-1 file. **[OURS] That error surface is exactly what an agent loop needs** — machine-readable,
specific, with the fix named. Nothing else in this survey is close on archival or accessibility
compliance.

PDF export is built on **`krilla` 0.8.2 (MIT OR Apache-2.0)** — confirmed from `typst-pdf`'s dependency
list.

#### 3.1.7 HTML export — not ready

Tracking issue typst/typst#5512 is **open** (since 2024-12-03), NLnet-funded; 0.15 added MathML and
multi-file bundle export. Running it today [MEASURED]:

```
warning: html export is under active development and incomplete
warning: page set rule was ignored during HTML export
warning: align was ignored during HTML export
```

Scope is deliberately "semantic HTML with no CSS"; CSS and EPUB are explicit future work. **Unusable for
production styling** — but it is the substrate for the Typst→docx path in §3.4, which is why its limits
matter there.

### 3.2 LaTeX — don't bundle it

**Tectonic** (`tectonic` 0.17.0, MIT, 2026-07-27) is the only viable embedded LaTeX — genuinely a library
(`tectonic::latex_to_pdf(latex) -> Vec<u8>`), and per Engine-Transfer-Bench (arXiv:2608.18329, 4,211
compiles per host across three OSes) it is **the most reproducible LaTeX engine measured: 96.3–97.2%
success, within 0.9 pp across platforms**, versus 12–20 pp variance for TeX Live-style engines.

**But the bundle is the problem.** The default remote bundle is **2,881,562,112 bytes ≈ 2.88 GB**.
Tectonic doesn't download all of it — it uses HTTP byte-range requests to fetch individual `.sty`/font
files on demand and cache them — which means **first run needs network**. You can ship a trimmed local
`.ttb` for full offline, but no published size for a report-grade trimmed bundle exists
[UNVERIFIED; low hundreds of MB is the estimate]. TinyTeX for comparison: 1.6 MB infra-only / **67 MB**
(~100 packages) / **210 MB** default / 1.9 GB full — and it is a subprocess dependency, not a library.
(TinyTeX's README says "GPL-2"; the authoritative TeX Live `LICENSE.TL` says TeX Live "has neither a single
copyright holder nor a single license" — an LPPL-dominant mix, commercially redistributable. The README
label is imprecise, not operative.)

SwiftLaTeX is AGPL-3.0 and dead since 2024-06; texlive.js is GPL-2.0 and dead since 2017.

**[OURS] Verdict: no.** 200 MB–2.9 GB, a network-dependent-or-hand-curated bundle, and a language whose
failure mode is a 200-line log for a missing `\usepackage`. Typst is smaller, faster, offline-clean and
Apache-2.0. *(LaTeX does beat Typst on raw LLM generation — see §3.5 — but not by enough to buy 200 MB
and a network dependency.)*

### 3.3 HTML → PDF, and what WKWebView actually does

This deserves a first-hand answer because the intuitive assumption is wrong, and because we already embed
WKWebView and already render charts as HTML.

Both relevant bindings are **already in `ui/desktop/src-tauri/Cargo.lock`** via wry — `objc2` 0.6.4,
`objc2-app-kit` 0.3.2, `objc2-web-kit` 0.3.2 [SOURCE]. So either path below costs **zero new
dependencies**.

**Path A — `createPDF` is a vector screenshot, not a print engine.**

```rust
// objc2-web-kit-0.3.2/src/generated/WKWebView.rs:716
#[unsafe(method(createPDFWithConfiguration:completionHandler:))]
pub unsafe fn createPDFWithConfiguration_completionHandler(
    &self,
    pdf_configuration: Option<&WKPDFConfiguration>,
    completion_handler: &block2::DynBlock<dyn Fn(*mut NSData, *mut NSError)>,
);
```

`WKPDFConfiguration` has **exactly two properties** [SOURCE, read from
`objc2-web-kit-0.3.2/src/generated/WKPDFConfiguration.rs`]:

- `rect: CGRect` — "The rect to capture **in web page coordinates**. If the rect is set to the null rect,
  the bounds of the currently displayed web page will be used."
- `allowTransparentBackground: bool` — default `NO`.

**That is the whole surface: no page size, no margins, no headers or footers, no pagination.** Apple's own
doc comment confirms it — with a nil configuration the method "will create a PDF document representing
**the bounds of the currently displayed web page**." It never runs WebKit's print layout pass, so it
ignores `@page`, `page-break-*` and `@media print` *by construction*, producing one enormous page. A
developer on Apple's forums tried slicing a long document by passing per-page `rect` values and found
**the rect is ignored — every call returns the full content** (forums thread 700418).

**Path B — `printOperationWithPrintInfo` is the real paginated path.**

```rust
// objc2-web-kit-0.3.2/src/generated/WKWebView.rs:964
#[unsafe(method(printOperationWithPrintInfo:))]
pub unsafe fn printOperationWithPrintInfo(&self, print_info: &NSPrintInfo)
    -> Retained<NSPrintOperation>;
```

`NSPrintInfo`, `NSPrintOperation` and `NSPrintPanel` are all in `objc2-app-kit` 0.3.2 [SOURCE]. Setting
`jobDisposition = .saveJob` with a job-saving URL, plus paper size and margins, does go through WebKit's
real print pipeline, and one developer report confirms "correct page count and sizing" (forums 705138) —
with the classic bug being `runOperation()` instead of `runOperationModalForWindow:`, which yields blank
pages. Full print-CSS fidelity through this path is **[UNVERIFIED]**. The costs are real: main thread, a
window/responder chain, and a UI-shaped API driven headlessly — a Rust daemon would have to marshal the
job into the Tauri process.

**And browsers cannot do running headers at all.** Basic `@page { size; margin }` landed in Safari 18.2 /
macOS 15.2, but **`@page` margin boxes (`@top-center` etc.) and `counter(page)`/`counter(pages)` are
supported by no browser engine** [VENDOR, MDN]. There is no CSS route to "Page 3 of 12" in any browser.

**The most instructive data point is an app shaped exactly like ours.** `kashioka/Rendu` is a **Tauri 2
macOS app** doing Markdown→PDF through its embedded WKWebView. Its issue #57: PDFs **silently truncate
past ~21 A4 pages**, root-caused to **WebKit's 16,384 px CoreAnimation layer cap**. They trialled paged.js
(MIT) to pre-paginate around it, hit CSS-across-page-boundary and table breakage, and **shipped an
HTML-export escape hatch instead of fixing PDF pagination.** paged.js's own CLI uses Puppeteer/Chromium;
the maintainers ship no WebKit path.

**Headless Chromium** has the best print-CSS fidelity of anything here (`headerTemplate`, `footerTemplate`,
`preferCSSPageSize`), at **93.4 MB compressed** for `chrome-headless-shell-mac-arm64` (179.2 MB for full
Chrome) [MEASURED, live HEAD 2026-09-01]. But Google states Chrome for Testing "has been created purely
for browser automation and testing purposes", and **redistribution terms for embedding a CfT binary in a
paid DMG could not be found** [UNVERIFIED — would need legal review]. `chromiumoxide` and
`headless_chrome` bundle no browser.

**Rust-native options:** `krilla` 0.8.2 (MIT/Apache) is PDF *construction* only and explicitly out-of-scope
for "text layouting, tables, page breaking, headers/footers"; `pdf-writer` sits under it; `printpdf`
0.12.7 has an **experimental** `from_html`; `lopdf` 0.44 (which we already use at 0.41) manipulates
existing PDFs; `blitz` renders HTML/CSS to wgpu with **no PDF export at all**. WeasyPrint has the best CSS
Paged Media support surveyed but needs Python ≥3.10 plus Pango/Cairo/GLib from Homebrew and ships no
standalone macOS binary — disqualified.

**[OURS] Verdict: WKWebView is not a paginated PDF engine. Use it to *preview* a PDF we produced
elsewhere — PDFKit in the existing web view — not to produce one.** `createPDF` is genuinely the right
tool for single-artifact vector capture (one chart, one diagram, one card, one poster, with optional
transparency), and that is a real capability worth having. It is not a document engine.

### 3.4 DOCX and slides

**DOCX: keep `docx-rs`, drive it from a structured model.**

| Crate | Version | Licence | Downloads | Verdict |
| --- | --- | --- | --- | --- |
| **`docx-rs`** (bokuweb) | **0.4.22** (2026-07-21) | **MIT** | 3.27M | **The mature choice — and we are already on 0.4.20.** Styles, tables (`GridSpan`/`VMerge`, borders, widths), images, headers/footers, page setup, breaks, hyperlinks, bookmarks, comments, footnotes, TOC, tracked changes. Reads and writes. README caveat: "OOXML is a large specification, so support is not exhaustive." |
| `docx-rust` | 0.1.11 | MIT | 2.24M | read-modify-write; table support undocumented [UNVERIFIED] |
| `rdocx` (tensorbee) | 0.11.1 | MIT/Apache | 6.3k | Six months old, 37★, 1,215/1,244 commits from one account with agent-shaped titles. **Pilot before trusting.** |
| `docx` (PoiScript) | — | — | 30k | **Abandoned since 2024-06** |

**[OURS] The upgrade is 0.4.20 → 0.4.22 and a change of *caller*, not of crate.** Today `docx_tool.rs`
takes imperative instructions (append / replace / insert / add image). The recommendation is to drive
`docx-rs` from the same structured content the agent produces for Typst, so the agent never touches OOXML
and there is no syntax-error surface at all.

**Typst → docx exists and is pure Rust, with a real limit.** `typlite` 0.15.4 (**Apache-2.0**, from the
tinymist project) converts Typst → md/txt/tex/**docx**, using `docx-rs` internally. Run on the test report
[MEASURED]: produced a valid 130 KB `.docx` in ~0.1 s with a **181-row `<w:tbl>`**, `styles.xml`,
`numbering.xml`, `footnotes.xml`, `comments.xml`. But `#set page(...)` is a **hard error** ("page
configuration is not allowed inside of containers"), `#align(center)` is silently dropped, and there are
only 5 `w:pStyle` references — because typlite routes through the experimental HTML target (§3.1.7).
**Practical consequence: you cannot take a print-tuned Typst source and get a docx from it.** One source
feeding both targets needs `#context if target() == "html" { … }` discipline.

**Typst will never export docx natively** — typst/typst#190 was closed `not_planned` in 2023:
"Direct output to docx is not planned."

**Worth watching, not shipping: `carta`** — "a fast, lightweight reimplementation of pandoc" in Rust,
**Apache-2.0 OR MIT**, macOS arm64 tarball **5.31 MB** (vs pandoc's 190 MB installed), with docx
reader+writer, Typst, LaTeX, HTML, EPUB, ODT, reveal.js and Beamer — but **no pptx**, and it is at
**0.0.10, 561 downloads, self-labelled "API is still unstable."** If it matures it is the GPL-free pandoc.

**Slides: Typst + `touying` → PDF.** `touying` 0.7.4 (**MIT**, 2,316★, pushed 2026-08-17, **109 KB**) —
six built-in themes, `#pause`/`#meanwhile`/`#uncover`/`#only`, handout mode, speaker notes, native PDF
with bookmarks. `polylux` 0.4.0 (MIT) last released 2025-01-27 and self-warns "no backwards compatibility
guarantees" — stale by comparison. Marginal bundle cost of touying is **~0**, because we already shipped
the compiler.

**The Rust pptx ecosystem is as weak as suspected**: `ppt-rs` 0.2.25 is the strongest (80k downloads,
49★, Apache-2.0) and is ten months old; `pptx-rs`, `pptx`, `deckmint`, `rpptx` are all sub-1.0 with
four-figure download counts. **None is production-grade.**

**And every HTML/Typst→pptx path rasterizes.** `touying-exporter` "generates PNG image files and packages
them into a PPTX" and needs Python + `python-pptx`. Marp's default pptx embeds slide images — "contents
cannot to modify or re-use in PowerPoint" — and `--pptx-editable` **additionally requires LibreOffice
Impress**. Slidev: "all the slides in the PPTX file will be exported as images", plus Playwright.
reveal.js → PDF via DeckTape needs Chrome. **[OURS] Be honest with users: PDF is the slide deliverable.**
If editable pptx is genuinely required later, drive `ppt-rs` from the structured slide model as a
secondary target — pilot it, don't assume it.

**Pandoc: 190 MB and a GPL question we don't need.** Pandoc 3.11 (2026-08-29) is **GPL-2.0-or-later** with
no commercial licence and none coming — John MacFarlane: "This is a dead issue. Even if I wanted to change
the license, I'd have to get approval from over a hundred contributors." The `.pkg` is 41.7 MB but the
**installed binary is 190,159,728 bytes ≈ 190 MB** [MEASURED]. On the subprocess question, the author's own
position in the same thread is favourable — "You might as well shell out to pandoc, which you could
distribute, together with its source code. **This way of using pandoc would not require you to GPL license
your code**" — and that matches the FSF's mere-aggregation FAQ. **[OURS] But this codebase already went to
the trouble of engineering espeak (GPL) out of the TTS path to get a "GPL-clean shipping backend."
Re-introducing a 190 MB GPL binary to do a job Typst does in 35 MB would undo that decision for no gain.**

### 3.5 Which markup should an agent author?

| Markup | Generation reliability | Error recovery | Token efficiency | Visual predictability |
| --- | --- | --- | --- | --- |
| **Markdown** | Highest — cannot fail to compile | n/a | Best | Poor: no page or layout control |
| **HTML/CSS print** | High — models have seen enormous volumes | Excellent: malformed HTML still renders | Verbose (2–3× Typst for the same layout) | **Poor for print** — no engine renders `@page` margin boxes or page counters; pagination is what models predict worst |
| **LaTeX** | Good syntax, **poor package/preamble reliability** | **Worst** — one bad `\usepackage` is a 200-line log | Verbose | Moderate; strong training priors |
| **Typst** | **Weakest of the compiled options** | **Best** — single-line, span-pointed errors with `hint:` lines, and a **70 ms** compile | Best of the compiled formats | Good — layout semantics are simple and local |
| **Raw OOXML** | Effectively zero | Corrupt file, no diagnostic | Catastrophic | None |

The honest evidence on Typst is mixed and worth quoting rather than summarising. **Negative:** "GPT 5 and
Gemini 2.5 Pro are usually unable to write compiling Typst code, getting it confused with Markdown…
the similarities to MD + MathJax result in LLMs hallucinating Typst"; "The models do not seem to currently
understand Typst's concept of **modes** (code, markup and math)"; a two-column US-letter prompt produced
**20 compile errors** where the same prompt in LaTeX worked. **Positive:** "Frontier LLMs work great with
Typst. (Have published multiple books using it)"; "It was agonizing directing Claude Code to create a
preso using LaTeX… I chose Typst."

The one quantified comparison is **TeXFix-Bench (arXiv:2608.07617, 2026-08-07)** — 10,437 instances,
7 LLMs, 48,651 attempts: "A pinned engine gate yields a 27.5-point intention-to-treat compile spread
(56.7–84.2%). **Typst is markedly harder than LaTeX and Markdown.**" It also found "13.6–18.5% of
compiling repairs materially alter document text… **Compile success alone overstates repair quality.**"
⚠️ Single-author preprint, not peer-reviewed, and the only quantified Typst-vs-LaTeX LLM comparison that
exists. There is **no official Typst `llms.txt`** — the proposal (#5840) was **closed and rejected** in
Feb 2025 by a core maintainer.

The failure mode is easy to reproduce. Writing the benchmark document by hand, `table.header([*#*], …)`
for a "#" column header produced `error: unclosed delimiter` + `error: unexpected star`, because `#`
starts a code expression in markup mode [MEASURED]. That is exactly the mode-confusion class the reports
describe.

**[OURS] The decisive insight is not which markup wins — it is that free-form generation is the wrong
architecture.** Three independent sources converge on the same fix: the HN user publishing books ("The
template itself takes **json as input**… It works very very well"); **SeaSlides (arXiv:2608.03298)** —
"Rather than authoring coordinates, inline styles, or raw SVG geometry, the model writes structured slide
content through reusable components… while **templates own layout, style, and rendering**"; and the
**Dual-Track Framework (arXiv:2606.23107)** — confining the LLM to reasoning-heavy parts and delegating
deterministic formatting to rules "achieves a higher compilation success rate."

**So: we author the Typst templates; the agent emits structured JSON into `sys.inputs` via
`compile_with_input()`.** Typst's weak priors stop mattering because the agent is barely writing Typst.
When it does need to — a custom table, a one-off layout — the **70 ms** compile plus a precise, hinted
`error:` line gives the tightest fix loop of any option here. **And the compiler is the validator: never
emit a document that didn't compile.** The same structured model drives `docx-rs`, where the agent touches
no markup at all.

Worth wiring as agent tools: `johannesbrandenburger/typst-mcp` (MIT, 175★, pushed 2026-08-24) exposes
`check_if_snippet_is_valid_typst_syntax`, `latex_snippet_to_typst` (models write LaTeX better → transpile),
`get_docs_chapter` and `typst_to_image`. Its own README says plainly: "LLMs are better at writing LaTeX
than Typst."
---

## 4. Orchestration — how to expose this to the agent

This section is mostly other people's hard-won design, borrowed deliberately. The abstraction question has
a settled answer in 2026 and we should not invent our own.

### 4.1 Tool shape: copy `ImageModelV3`, copy `action: generate|edit`

**The best cross-provider abstraction in existence is the Vercel AI SDK's `ImageModelV3`** [VENDOR,
`packages/provider/src/image-model/v3/image-model-v3.ts`]:

```ts
interface ImageModelV3 {
  specificationVersion: 'v3';
  provider: string;
  modelId: string;
  maxImagesPerCall: number | undefined | (() => …);
  doGenerate(opts: ImageModelV3CallOptions): PromiseLike<{
    images: Array<string> | Array<Uint8Array>;
    warnings: Array<SharedV3Warning>;
    providerMetadata?: Record<string, …>;
    response: { timestamp: Date; modelId: string; headers?: … };
    usage?: ImageModelV3Usage;
  }>;
}
// ImageModelV3CallOptions:
//   prompt: string | undefined     ← undefined is legal (upscale / variation ops)
//   n: number
//   size: `${number}x${number}` | undefined
//   aspectRatio: `${number}:${number}` | undefined
//   seed: number | undefined
//   providerOptions: SharedV3ProviderOptions
```

Five design decisions in there are worth stealing verbatim:

1. **Only five portable knobs** — prompt, n, size, aspectRatio, seed. Quality, background, style,
   moderation, fidelity all go in `providerOptions.<provider>` and are **deliberately not normalized**.
   Trying to normalize `quality` across providers is a known trap; the SDK refuses on purpose.
2. **`size` and `aspectRatio` are separate and mutually exclusive per provider.** OpenAI takes `size`;
   Google, fal, xAI and BFL take `aspectRatio`. A portable layer must model both, not pick one.
3. **Warn, don't throw, when a provider ignores a setting.** An image generated at the wrong aspect ratio
   is money already spent; hard-failing converts a degraded result into a total loss.
4. **`maxImagesPerCall` + transparent fan-out** to satisfy `n`, with per-call usage/warnings preserved.
5. **Middleware** (`wrapImageModel`) is the insertion point for caching, budget checks and prompt
   rewriting — not the model implementations.

LangChain and LlamaIndex have **no equivalent settled abstraction**; image generation there is a per-provider
tool wrapper. The AI SDK is the only mature cross-provider media abstraction as of Sept 2026. We are in
Rust, so we port the shape, not the code.

**And copy OpenAI's `action` parameter.** Its `image_generation` built-in tool takes:

```
model, size, quality, background, format, compression,
partial_images (0-3), action: "auto" | "generate" | "edit",
moderation, input_fidelity
```

`action` is the published answer to the generate-vs-edit ambiguity, and **[OURS] it is the single most
important knob in the whole design.** Do not let an LLM infer from prose whether "make it bluer" means a
fresh $0.21 render or an edit of the last image — the cost delta between those two readings is the whole
product.

**Carry image identity by reference, never by bytes.** OpenAI: `previous_response_id`, or
`{"type": "image_generation_call", "id": "<prior call id>"}` in the input array. Google:
`previous_interaction_id` + thought signatures. Re-uploading reference images inflates input tokens on
every turn of a refine loop.

**Return file paths, not base64.** This is the near-universal MCP community convention (mikeyny/ai-image-gen-mcp,
GMKR/mcp-imagegen, fal-mcp all return saved paths plus `inference_time_ms`), and it keeps multi-megabyte
payloads out of the agent transcript. It also matches how our `autovisualiser` already hands back
resources.

### 4.2 Prompt expansion — helps, but we are already stacking on a hidden rewrite

**Providers rewrite prompts whether we like it or not.**

- **OpenAI**: "the mainline model automatically revises prompts for improved performance," exposed as
  **`revised_prompt`** on the image_generation_call. Not optional; we only get to observe it [VENDOR].
- **Google Veo**: an LLM prompt rewriter is **on by default**; `enhance_prompt: false` disables it and
  Google warns quality drops. Critically, **the rewritten prompt is returned only if the original was
  under 30 words** — above that we are flying blind [VENDOR]. Imagen had the analogous `enhancePrompt`.
- **Gemini image models**: no documented statement either way.

**Research says expansion helps, with a measurable trade-off.** [Input-Side Inference-Time Scaling,
arXiv 2510.12041] trains a rewriter via iterative DPO:

| Benchmark | Baseline | + rewriter |
| --- | --- | --- |
| GenEval (FLUX.1-dev) overall | 0.70 | **0.79** |
| GenEval *position* | 0.22 | **0.58** |
| FID, MS COCO-30K (FLUX.1-dev) | 24.38 | **19.57** |

But an **aesthetics-tuned rewriter reaches 0.818 aesthetics win-rate while dropping alignment to 0.424**,
versus a general rewriter at 0.561 alignment / 0.476 aesthetics — "aesthetics reward often enriches scenes
by introducing additional objects or ornamentation, which can dilute the main subject." Separately,
rewriting **degrades affective/emotional control** [EPIG, arXiv 2606.13247].

**[OURS] Rules that follow.** Expand only when the user's prompt is short and under-specified — that is
where the documented gains come from. Skip expansion when the prompt is already long or emotionally
specific. Prefer a *general* rewriter over an aesthetic one. Always surface `revised_prompt` and let the
user pin it. And remember the real risk is **double expansion**: our rewrite on top of the provider's
undisclosed one.

### 4.3 Review-before-spend

**The numbers make this mandatory, not nice-to-have.** A sub-cent local draft, a $0.03 FLUX.2 pro image, a
$0.21 gpt-image-2 high, and a **$7.00 ten-second Sora 2 Pro 1080p clip** are four orders of magnitude
apart, and the same agent can call all of them.

What providers actually give us:

| Provider | Pre-flight estimate | Post-hoc cost | Hard cap |
| --- | --- | --- | --- |
| **fal.ai** | ✅ `GET /v1/models/pricing?endpoint_id=…` + a dedicated estimate endpoint | ✅ `x-fal-billable-units` header × unit price | prepaid credits |
| Replicate | ❌ | ⚠️ `metrics.predict_time` — **no dollar field**; multiply by hardware rate yourself | account monthly spend limit |
| OpenAI | ❌ | ✅ full `usage` breakdown incl. `input_tokens_details` | project usage limits |
| Google | ❌ | via token accounting | GCP budgets are **notify-only by default**; the opt-in "spend cap" budget type genuinely pauses |
| BFL | ✅ — the submit response includes **`cost`, `input_mp`, `output_mp`** | same | prepaid credits, 402 when out |
| Runway | ⚠️ balance via `GET /v1/organization` | credits | auto-billing |

**[OURS] Note that BFL is the one that hands us the cost in the submit response.** For OpenAI images,
Sora, Veo and Replicate there is **no dry-run endpoint at all** — the only workable pattern is a
price table in our own code, multiplied by requested quality/duration/resolution, gated before submit.

**The gate mechanism has three good precedents:**

- **OpenAI Agents SDK** — `needsApproval: true`. The SDK **records an interruption and pauses the run**,
  giving you `interruptions` plus a **serializable, resumable `state`**: "If the review might take time,
  serialize `state`, store it, and resume later. That's still the same run." Their guidance explicitly
  says put the gate **next to the tool that creates the side effect**, not at agent level [VENDOR].
- **LangGraph `interrupt()`** — checkpointer-persisted, with approve / **edit** / reject / respond.
  The `edit` decision matters here: a human should be able to *downgrade quality or trim seconds*, not
  only say yes or no.
- **MCP elicitation** (spec 2025-06-18) — `elicitation/create` with a **flat, primitive-only**
  `requestedSchema` and a three-action response: `accept` / `decline` (explicit no) / `cancel`
  (dismissed). Servers **MUST NOT** request sensitive info. This is the protocol-native way for an
  image-gen MCP server to say "this clip costs $7.00 — proceed?" and to tell "no" apart from "went away."
  Client adoption was still uneven in early 2026.

**Put a threshold on the gate.** A single flat "confirm every generation" rule trains the user to click
through the one that matters. Sub-cent should never prompt; a multi-dollar video always should.

**And back the interactive gate with a server-enforced budget that actually rejects.** Alerts are not
enforcement. Field evidence: CockroachDB reports inference spend going **$12k → $68k in six weeks** from a
retrieval fault causing 8× over-fetch, and recommends **escalating to a human after a retry-count
threshold** rather than retrying indefinitely.

### 4.4 Safety, provenance, and what the law now requires

**Every provider pushes liability downstream to us.** OpenAI's Services Agreement makes the customer
responsible for "all activities that occur under its Account, including the activities of End Users," and
indemnifies OpenAI for Customer Applications and Customer Content; OpenAI indemnifies only IP claims
against the Services themselves, **not the content the API generates**, with liability capped at trailing
12-month spend. Replicate is more extreme: §8.5 "**Replicate does not monitor or police** … outputs
generated," §8.6 output "is Customer's to manage," liability capped at the **lesser of trailing-6-month
spend or $100**.

**Moderation is not contractually required, but abuse-attribution is expected.** OpenAI: "OpenAI's
Moderation API is free-to-use… Alternatively, you may wish to develop your own content filtration system."
What they do push: human review before production, user registration, an easy abuse-reporting channel, and
sending a stable hashed **`safety_identifier`** so they can detect abuse. **[OURS] We should send one.**

**Provenance.** OpenAI attaches C2PA Content Credentials *and* an invisible watermark to supported images
from ChatGPT, Codex "**and the OpenAI API**" [VENDOR], while conceding metadata "can sometimes be removed
by platforms, editing tools, or file conversions" — an acknowledged limitation, **not stated as a
violation**. Google's SynthID is automatic across Gemini/Imagen/Lyria/Veo with **no documented developer
opt-out**; detection is waitlist-gated, not a public API; Google's materials never mention C2PA. BFL
reserves the right to embed Content Credentials without notice. **[OURS] Whatever the contracts say,
preserve it: don't re-encode, don't strip EXIF wholesale, don't thumbnail-and-discard-the-original.**

**EU AI Act Article 50 is live — as of 2 August 2026 — and was NOT delayed.** The Digital Omnibus
(Regulation (EU) 2026/1744, in force 27 July 2026) pushed the *high-risk* Annex III obligations to
2 Dec 2027 and Annex I to 2 Aug 2028, but **left Article 50 intact**:

| Obligation | Deadline |
| --- | --- |
| 50(1) interaction disclosure, 50(3) emotion/biometric, **50(4) deepfake disclosure** | **2 Aug 2026 — live** |
| 50(2) machine-readable marking, systems placed on market on/after 2 Aug 2026 | **2 Aug 2026 — live** |
| 50(2) marking, systems already on market before that date | **2 Dec 2026** (grace period, expiring) |
| Enforcement / fining powers | **2 Aug 2026 — live** |

Penalties reach **€15M or 3% of worldwide turnover**. No binding harmonised marking standard exists yet;
the Commission published a **voluntary Code of Practice on transparent generative AI on 10 June 2026**.

**[OURS] The load-bearing point: upstream C2PA/SynthID discharges the *provider's* 50(2) duty. It does not
discharge our independent 50(4) *deployer* duty to disclose.** If Permagent publishes synthetic media into
the EU, we owe a disclosure of our own. Artistic/satirical/fictional work gets a lighter touch — disclosure
"in an appropriate manner that does not hamper the display or enjoyment of the work" — not an exemption.
AI-generated *text* on matters of public interest also needs disclosure unless a human reviewed it and
holds editorial responsibility.

Likeness rules worth knowing: **Sora is opt-in only** — it cannot generate anyone, public figures
included, unless they have submitted a verified "cameo"; estates and representatives can request
exclusion [VENDOR]. OpenAI bans political campaigning and election interference outright, and has by far
the most granular CSAM/grooming prohibitions ("whether or not any portion is AI generated"). Google's
likeness policy could not be confirmed in primary text [UNVERIFIED].

### 4.5 Retention windows — the ten-minute rule

| Provider | Delivery | Expiry |
| --- | --- | --- |
| **BFL FLUX** | signed URL from `polling_url` | **10 minutes** |
| **OpenAI Sora** | `GET /videos/{id}/content` | **1 hour** |
| **Replicate** | `replicate.delivery` URL | **1 hour** (API predictions) |
| **Google Gemini Files API** | `files.download` | **48 hours**, 20 GB/project, 2 GB/file, free |
| OpenAI gpt-image | base64 inline | n/a — we already hold the bytes |

**[OURS] Ten minutes is the binding constraint and it dictates architecture.** Any design that hands a
provider URL to the UI, or defers persistence to a background job, is broken against BFL and fragile
against Sora and Replicate. **Download inside the same request that observes completion**, write to
`Paths::data_dir()/media/`, and hand the agent a local path.

### 4.6 Caching, idempotency, progress

**No provider offers an idempotency key for media generation.** Seeds are the only reproducibility handle,
and are honoured unevenly. A content-addressed cache is entirely ours, keyed on
`(provider, model, final_prompt_after_rewrite, size|aspectRatio, quality, seed, reference_image_hashes)` —
and **the post-rewrite prompt must be inside the key**, or we cache-miss on every call whose provider-side
rewrite differed.

Three progress shapes exist and an agent product needs all three:

1. **Token-stream partials** — OpenAI only. `partial_images: 1-3` + `stream: true`, events carrying
   `partial_image_index` / `partial_image_b64`. **+100 image output tokens per partial** — a deliberate
   purchase of perceived latency.
2. **Long-running operation + poll** — Sora, Veo, BFL, fal, Replicate. **Sora is the only one exposing a
   `progress` percentage.** BFL statuses are `Pending | Ready | Error | Failed`, and you **must** use the
   returned `polling_url` rather than constructing one, because of cross-cluster routing. fal exposes
   `queue_position`.
3. **Webhooks** — Sora, Replicate, fal, Ideogram (Ed25519-signed), BFL. The right choice for anything
   over ~30 s, with polling as fallback.

Billing invariants worth exploiting: **fal charges nothing for HTTP 5xx or queue wait; Google charges
nothing for failed video renders; Seedream charges only on success.** So retry-on-server-error is cheap.
**Retry-on-bad-output is not, and retry-on-moderation-block is never right** — distinguish
`moderation_stage: input` (user-fixable, surface an editable prompt) from `output` (not fixable, do not
silently retry at cost).
---

## 5. Recommendation matrix

Primary = what the agent should reach for by default. Local alternative = what runs on a 16 GB Mac with
weights we may legally redistribute. Cost is per unit of output.

| Use case | Recommended primary | Local alternative | Cost / unit | Why |
| --- | --- | --- | --- | --- |
| **Illustration / general image in a document** | **FLUX.2 [pro]** (BFL) | FLUX.2-klein-4B Q4_K_S via sd.cpp+Metal (~5.0 GB download) | **$0.03** cloud · $0 local | Best quality-per-dollar at Elo 1208; BFL is the only provider returning `cost` in the submit response, and its terms give us the output outright. **Same model family as the local fallback, so prompts transfer** — the strongest argument for this pairing. |
| **Image with text in it** (poster, UI mock, diagram label) | **gpt-image-2 (high)** | **none — do not attempt locally** | **$0.211** | The 237-Elo gap concentrates here. gpt-image-2's reasoning pass is the only one that renders paragraphs and UI reliably. Ideogram 4.0 at $0.06 is the value pick if $0.21 is too rich. |
| **Cheap draft / iteration pass / thumbnail** | **local first** — FLUX.2-klein-4B Q3_K_S | **the local default** (~4.2 GB) | **$0** | Drafts are where local earns its keep: no per-image cost, no egress, and quality barely matters at thumbnail scale. Escalate to cloud only when the draft is promoted. |
| **Transparent-background asset** (icon, sticker, overlay) | **gpt-image-2** `background:"transparent"` | none | $0.053 med / $0.211 high | Only OpenAI gives alpha for free in the same call. Ideogram has dedicated endpoints at the same 1K price; Google, BFL, xAI, Luma, Seedream and Qwen have **none**. |
| **Editable / brand-controlled vector asset** | **Recraft V4.1 vector** | none | **$0.08** (SVG) | An SVG we can restyle in code beats a raster we can't. `+$0.01` vectorize, `+$0.01` remove-background. |
| **Any edit of a user's own image** | **OUT OF SCOPE — no cloud path exists in the build** | none on 16 GB (§1.1.4) | — | Jesse's ruling: personal photos never leave the machine. Local editing is unavailable at this hardware tier, so **image editing is not a v1 capability**. Say so in the product rather than letting a user discover it. |
| **"Remove the background / clean up this photo"** | **Apple Vision + Core Image, on-device** | same — it *is* local | $0 | `VNGenerateForegroundInstanceMaskRequest` does subject lifting entirely on-device; Core Image covers crop/scale/filter. Not generative, no egress, and the right answer for most of what users mean by "edit". |
| **Offline / sovereign mode** | — | **FLUX.2-klein-4B Q4_K_S** (Apache-2.0, ~5.0 GB) | $0 | Local is unambiguously primary. Must route through `guard_outbound_egress` like everything else, and be labelled honestly. |
| **Video, any use case** | **none — stub it** | **none — 16 GB cannot** | $1.20–$4.00 per 10 s 1080p | One clip eats 12–40% of a task's $10 hard ceiling. Sora's API dies 24 Sep 2026; Google swapped its own default a week ago. When a use case appears: `gemini-omni-1.1-flash` at $0.101/s, or fal as one key. |
| **PDF report** | **Typst 0.15.1 embedded** (Apache-2.0, +25–40 MB) | same — it *is* local | $0 | 70–90 ms compiles, real pagination, repeating table headers, PDF/A + tagged PDF, offline packages, all fonts redistributable. Nothing else here does running headers at all. |
| **Slide deck** | **Typst + `touying` 0.7.4 → PDF** | same | $0 | MIT, 109 KB, ~zero marginal cost once the compiler ships. Every HTML→pptx path rasterizes; be honest that PDF is the deliverable. |
| **Word document (.docx)** | **`docx-rs` 0.4.22**, driven from the same structured model | same | $0 | Already at 0.4.20 in `goose-mcp`. The change is the *caller*, not the crate — the agent should never touch OOXML. |
| **Single artifact → vector PDF** (one chart, card, poster) | **WKWebView `createPDF`** | same | $0 | Zero new dependencies (`objc2-web-kit` already in the lockfile), vector output, optional transparency. Reuses the `autovisualiser` HTML we already render. |
| **Charts inside a document** | **`autovisualiser` HTML → SVG → embed in Typst** | same | $0 | Bridges what we already built. `lilaq` 0.6.0 (MIT) is the native-Typst option later. **Avoid `cetz` — LGPL-3.0.** |
| **Long paginated HTML → PDF** | **don't** — render via Typst instead | — | — | `createPDF` has no pagination (two config properties: `rect`, `allowTransparentBackground`); `printOperation` works but is main-thread + window-bound; WebKit truncates past ~16,384 px. |
| **Apple Image Playground** | **not usable** | — | free | `ImageCreator` stops compiling in macOS 27; the replacement is a modal sheet only; and it now runs on Private Cloud Compute, so it is **not on-device anyway**. Apple's own migration note says to integrate another service. |

**[OURS] If only three things get built, build these:** Typst embedded as the document engine;
**FLUX.2-klein-4B locally via sd.cpp+Metal** behind a provider trait, with BFL FLUX.2 pro as the cloud
escalation on the same prompt shape; and `EgressKind::MediaGeneration` wired through
`guard_outbound_egress` plus a USD path into the budget ledger.

Note that the ordering has changed under the ruling. Cloud editing is gone, so the cloud tier is now a
single text-prompt call that any provider can serve — a day's work, and deferrable. **Local generation is
no longer the second milestone; under a task-based split where local owns drafts, offline, privacy and
anything touching the user's images, it is the one that carries the product.** Its risk is also the one
that needs retiring earliest: the half-day Metal spike in Trap 3 should happen before anything else in
this document is built.
---

## 6. The top three traps

### Trap 1 — Media generation is a new egress path, and the sovereignty guard does not cover it

`SovereignGuardProvider` is described in its own doc comment as "the sovereignty enforcement choke point…
Because every `Arc<dyn Provider>` in the daemon is produced there, wrapping at that single point makes this
**the one place all inference egress passes through**" [SOURCE,
`crates/goose/src/providers/sovereign_guard.rs`]. It works by intercepting exactly two trait methods:
`Provider::stream` and `Provider::create_embeddings`.

**A cloud image call is neither.** And `EgressKind` is a closed enum — `Inference`, `Embedding`,
`Telemetry`, `CodeScan`, `CrashReport` [SOURCE, `crates/goose/src/sovereignty/mod.rs:285`]. **There is no
variant for media.** Wire an image provider as an ordinary HTTP client and we will have built a path that
ships the user's prompt — and, on an edit call, the user's *photograph* — off the machine with **no
`DataLocality` check, no fail-closed sovereign refusal, and no row in the egress audit log**. That is a
silent regression in the strongest privacy guarantee the product makes, introduced by a feature nobody
would think to audit for it.

The fix is small and already scaffolded: add `EgressKind::MediaGeneration` (and, because an edit uploads
user content rather than just a prompt, arguably a distinct `MediaUpload`), and call the existing
`guard_outbound_egress(kind, destination, label) -> bool` before every cloud media request — the function
whose doc comment already anticipates exactly this ("no ambient path exists today; a future one **must**
flow through the same boundary"). It is fail-closed and it refuses even an *allowed* call if the audit row
cannot be written.

**Do this in the first commit, not as a follow-up.** It is three lines at the call site and it is the
difference between a feature and a breach.

### Trap 2 — Media spend is invisible to the existing budget ledger

Permagent already has the spend governance most products lack: `providers::canonical::cost::cost_of`
rolls every response's USD into `sessions.accumulated_cost_usd`, and `cost_router::budget` turns that into
SOFT / GATE / HARD bands at task $2/$5/$10 and session $10/$25/$50, where GATE raises a **fail-closed
Tier-2 `choice` in the Decision Inbox** that only the user can answer [SOURCE,
`crates/goose/src/cost_router/budget.rs`].

But `cost_of(usage: &Usage, pricing: &Pricing)` is **purely token-based**, and `Pricing` has only
`input` / `output` / `cache_read` / `cache_write`, all per-million-tokens [SOURCE,
`crates/goose/src/providers/canonical/model.rs:37`]. There is no per-image or per-second field.

So the failure mode is specific and quiet: OpenAI and Google image calls *do* return token usage and could
be made to flow through the ledger with a `Pricing` row, but **BFL, Ideogram, Recraft, Stability, and every
video provider bill flat per-image or per-second and report no tokens at all**. Those calls would run up
real money while `accumulated_cost_usd` stays flat, the budget bands never trip, and the Decision Inbox
never asks. A user with a $10 session ceiling could spend $70 on ten Sora clips without a single gate
firing.

The fix: give the ledger a direct USD write path for non-token spend, and record media cost there
*before* the generation is dispatched where the price is deterministic (it almost always is — quality tier,
or seconds × resolution rate). BFL helpfully returns `cost` in the submit response; for everyone else it is
our own price table. Then set a **threshold** on the gate so sub-cent local drafts never prompt and
multi-dollar video always does — a flat "confirm everything" rule trains the user to click through the one
that matters.

### Trap 3 — Assuming the obvious runtime is the working runtime

Three separate versions of this trap are sitting in the current state of the art, and each one looks like
the free answer right up until you build on it:

- **`candle` is already in our lockfile and already has `flux`, `mmdit`, `stable_diffusion` and
  `Config::schnell()` with the `metal` feature enabled** [SOURCE, verified in
  `~/.cargo/registry/.../candle-transformers-0.11.0`]. It reads as zero-cost. But the diffusion examples
  have not moved since 2024, there is no FLUX.2 / Z-Image / Qwen-Image support, and
  candle #2406 — a Metal FLUX failure — **has been open since August 2024** with reporters confirming it on
  M1 and M3. Choosing candle means owning a two-year-old implementation on a broken backend.
- **`libonnxruntime.dylib` (27 MB) and the `ort` crate are already in the bundle**, and the Kokoro TTS
  backend proves the pattern works beautifully for speech. It is natural to assume diffusion can ride the
  same rails. It cannot: `apple/ml-stable-diffusion` last committed **2025-07-03**, DiffusionKit was
  **archived 2026-03-21**, and no maintained ONNX path for modern DiT models on macOS exists.
- **`stable-diffusion.cpp`'s official macOS binary is CPU-only** — `CMakeLists.txt:90` is
  `option(SD_METAL … OFF)` and the macOS CI job never passes `-DSD_METAL=ON` [SOURCE]. Download the
  release, benchmark it, conclude local generation is hopeless, and you will have measured the wrong thing.
  Users are already making this mistake in the issue tracker.

The shared shape: **the thing that is already in the tree is not the thing that works.** Budget for a
half-day spike that builds sd.cpp with `-DSD_METAL=ON` and measures FLUX.2-klein-4B Q4_K_S at 1024² on the
16 GB M4, with and without VAE tiling, recording wall-clock, peak RSS and swap — before any integration
work. That specific cell (16 GB M4 × Metal × klein-4B × sd.cpp) is **empty in the public record**; every
number in §1.1.3 is a different chip, a different runtime, or CPU-only.

*Honourable mentions, each of which has already cost somebody a sprint:* BFL's result URLs expire in
**10 minutes**; Apple's `ImageCreator` **stops compiling in macOS 27**; Google **shut Imagen down on
17 Aug 2026**; FLUX.2 **removed** the mask parameter FLUX.1 Fill had; Ideogram's rate limit is a
**10-inflight concurrency cap**, not an RPM; and Ollama's image generation was **removed in v0.32.6** while
`/api/tags` still advertises `"capabilities": ["image"]`.

---

## 7. Open questions only Jesse can answer

These are decisions, not research gaps. Each one changes the recommendation.

### 7.0 Settled — recorded 2026-09-01

| Question | Ruling | Where it landed |
| --- | --- | --- |
| Cheapest viable local option, rather than a cloud spend threshold | **FLUX.2-klein-4B Q3_K_S + Qwen3-4B Q3 ≈ 4.2 GB** is the floor; Q4_K_S ≈ 5.0 GB is the quality-safe default. Nothing smaller is both redistributable and runnable without Python. | §1.1.5 |
| May personal photos leave the machine? | **Never. Cloud photo-editing is out of scope entirely** — not gated, not built. | §1.1.6, matrix |
| Local as default or cloud as default? | **Task-based split.** | §1.1.6, matrix |
| Is a ~5 GB first-run download acceptable? | **Approved.** | §1.1.5 |

The consequence worth re-reading is in §1.1.6: because local editing is unavailable on 16 GB, "photos stay
local" means **image editing is not a v1 capability at all**. Apple's Vision framework covers the
non-generative cases (background removal, subject lifting) entirely on-device.

### 7.1 Still open

1. **Spend appetite per image — now a smaller question, but not zero.** Cloud is text-prompt generation
   only, so the exposure is bounded. Still: at $0.03 (FLUX.2 pro) a $10 task ceiling buys ~65 images; at
   $0.211 (gpt-image-2 high) it buys 9. **When a local draft is promoted to a cloud final, does that
   escalation need a confirmation, or just a ledger entry?** Our recommendation is auto-approve below
   ~$0.05 and confirm above it.

2. **Spend appetite per video — and whether video is in scope at all.** A 10-second Sora 2 Pro 1080p clip
   is **$7.00**, i.e. 70% of a whole session's hard ceiling in one call. **Is there any use case where
   Permagent should spend that, or is video a stub behind the abstraction?** Our §2 verdict is "stub", but
   if you have a real use case in mind it changes the priority.

3. **Where exactly is the task-based line drawn?** The ruling settles *that* the split is task-based;
   it does not settle every cell. Clear: drafts, offline, privacy mode and anything touching a user image
   are local; text-in-image and "final deliverable from a text prompt" are cloud. **Ambiguous: a
   1024² illustration for a report the user is about to send.** Local costs $0 and 30–90 s at Elo 1133;
   cloud costs $0.03 and ~10 s at Elo 1208. **Does "it's going in a document someone else will read" make
   it a cloud task by default?**

4. **What happens before the ~5 GB download completes?** It is approved, but it cannot be instant.
   **Is image generation simply absent until the download finishes, does it fall back to cloud in the
   interim (which the task-based split would otherwise forbid for some tasks), or do we prompt at first
   use?** Related: the M1 headless mini is a poor fit for local generation (§1.1.3) — **does it get the
   download at all, or does that box route to the M4?**

5. **Do we say "image editing is not supported" out loud?** §1.1.6's honest consequence is that v1 has no
   editing capability of any kind. **Should the agent have an explicit tool that declines and explains —
   pointing at the on-device Vision path for background removal — or should editing simply not appear?**
   Silent absence is how users conclude a product is broken.

6. **Does Permagent publish into the EU?** EU AI Act Article 50(4) has been live since **2 Aug 2026** and
   imposes a **deployer** disclosure duty that upstream C2PA/SynthID does not discharge, with penalties up
   to €15M or 3% of turnover. **If any user might publish generated media into the EU, we owe a disclosure
   affordance in the product.** That is a small UI feature if decided now and an expensive retrofit later.

7. **Whose keys?** Everything in §1.2 assumes we hold provider credentials or the user does. **BYO-key,
   Permagent-billed, or both?** BYO-key removes our exposure to OpenAI's "you are responsible for all
   End User activity" indemnity and to Replicate's $100 liability cap, but makes onboarding worse and
   makes cost gating advisory rather than enforceable.

8. **Document engine: one or two?** §3 offers a genuine fork — a single Typst-based engine that produces
   PDF, slides and print output from one markup, versus keeping the existing `docx-rs` writer and adding
   an HTML→PDF path that reuses the `autovisualiser` charts we already have. **Doing both is the expensive
   answer and is also, right now, the accidental answer** — we already write DOCX one way and render
   charts another. Which one is the spine?
