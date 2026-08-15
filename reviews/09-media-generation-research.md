# Image and video generation for Permagent: what is actually viable

Researched 2026-08-15. Question: can Permagent generate image and short-video content for a project's social posts, using open-source components?

**Verdict: image yes, generative video no. And for "we shipped feature X", generative video is the wrong tool even where it works — it cannot render your UI, your text, or your brand. Compose the video instead of generating it.**

---

## 0. The machine decides the answer

Measured rather than assumed, on the dev box:

| | |
|---|---|
| Chip | **Apple M4 (base), 10 GPU cores** — not Pro, not Max |
| Memory | **16 GB unified** |
| Free disk | **17 GB of 460 GB** at time of measurement |
| ffmpeg | present, but Homebrew's **GPL** build |
| VideoToolbox | `h264_videotoolbox`, `hevc_videotoolbox`, `prores_videotoolbox` all present |

Every published benchmark uses an M4 Max with roughly 4× the GPU cores and 2× the bandwidth. **Treat third-party timings as a floor, not an estimate.**

---

## 1. Image generation — viable, Apache-2.0 available

| Model | Params | Weights licence | Disk (quantised) |
|---|---|---|---|
| **Z-Image-Turbo** (Tongyi/Alibaba) | 6B | **Apache-2.0** ✅ | 6.5 GB |
| **FLUX.2-klein-4B** (BFL) | 4B | **Apache-2.0** ✅ | 4.6 GB |
| ERNIE-Image (Baidu) | 8B | Apache-2.0 ✅ | ~5 GB |
| Qwen-Image | 20B | Apache-2.0 ✅ | ~12 GB — too big here |
| FLUX.2-dev, klein-**9B**, Ideogram 4, Krea, FIBO | — | ❌ non-commercial / restrictive | disqualified |
| Stable Diffusion 3.5 | — | ⚠️ Stability Community, **$1M revenue cap** | avoid |

**FLUX caution:** BFL announced FLUX.2 [klein] as Apache-2.0 but shipped only the **4B** that way. The 9B question is open and unanswered ([flux2#32](https://github.com/black-forest-labs/flux2/issues/32)). Do not architect assuming it follows.

**Serving:** **mflux (MIT)** on MLX, behind a thin local HTTP wrapper — the established "long-lived local service, Rust is a client" pattern (`picker.rs`). Alternatives and why not:
- `stable-diffusion.cpp` / `sd-server` (MIT) has an excellent async-job HTTP API, but **the prebuilt macOS binary is CPU-only** (`SD_METAL` defaults OFF and macOS CI never enables it).
- Draw Things and ComfyUI are both **GPL-3.0** — a bundling problem for a signed .dmg.

**Speed:** measured 105 s/image for Z-Image-Turbo at 1024×1024 via ComfyUI/MPS on an **M4 Pro**; ~30–40 s via mflux on an M1 Max. Inferred for base M4: tens of seconds via MLX, minutes via MPS. **Nobody has published a base-M4 number — measuring it is genuinely new information.**

---

## 2. Video generation — three independent walls

### Licences eliminate most of the field first

| Model | Weights licence | Verdict |
|---|---|---|
| **Wan 2.1 / 2.2** | **Apache-2.0** ✅ | the only clean option |
| **LTX-2.3 / 2.5** | ❌ LTX-2 Community | disqualified — see below |
| **HunyuanVideo** | ❌ Tencent Community: **territory excludes EU, UK, South Korea** | disqualified for an internationally distributed app |
| MiniMax, CogVideoX | ❌ restrictive | disqualified |

**On LTX**, from the licence text rather than the summaries. It is materially worse than the "$10M revenue cap" everyone quotes:
- **§3.1** — you must propagate the use restrictions into **your own EULA** as an enforceable provision, and notify downstream users.
- **§6** — you may not remove watermarking, must use the latest version, and **Lightricks reserves the right to remotely restrict usage**.
- **Attachment A #20** — no use in a product that "directly competes with Licensor's commercial products." **Lightricks ships LTX Studio and Videoleap.** A tool generating social video clips plausibly competes.

That is the same class of problem as the OpenMontage AGPL rejection, arriving through a different door.

### The hardware says no regardless

- **Disk:** LTX-2.5's base install is 27.5 GB (57.5 GB with the Q8 pack). Against 17 GB free it does not fit at all. Wan 2.2 TI2V-5B Q4 + VAE + umt5-xxl ≈ 8.5 GB is the only stack that fits.
- **Speed:** the best measured configuration anywhere — Wan 2.2 TI2V-5B, INT8, 3-step distillation — is **151 s for 3.4 s of video on an M4 Max 36 GB**, and that PR is unmerged and self-reported. Inferred for base M4: **8–12 minutes per clip.**
- **Silent corruption, the risk that actually matters.** Open verified issues show Mac stacks producing wrong output while reporting success: PyTorch MPS silently corrupting attention above 2³¹ elements (and the existing safeguard keys on *free memory*, so more RAM fails more silently); MLX returning all-zero tensors on command-buffer OOM; one report of **39 hours on an M1 Ultra producing entirely black frames**. Shipping this means permanently shipping frame-validation heuristics as a tax.

### The vendor's own verdict

**LTX Desktop — Lightricks' signed Mac app — requires ≥15 GB *free* RAM for local generation and silently falls back to their paid cloud below that.** The company that makes the model routes 16 GB Macs to the cloud in its own product.

**If generative video is wanted, cloud is the deliberate choice:** fal.ai ≈ $0.05–0.40 per video-second (~$0.30/clip), delivered in under a minute.

---

## 3. Composition beats generation for this use case

The artefact actually wanted for "we shipped X" is a screenshot in a branded frame, a code diff, animated text, a logo sting. Generative video cannot produce any of those.

**The key unlock: FFmpeg's VideoToolbox encoders carry no GPL dependency.** Verified against FFmpeg's `configure` — `h264_videotoolbox`, `hevc_videotoolbox`, `prores_videotoolbox`, plus `drawtext`, `zoompan`, `xfade`, `overlay`, `subtitles`/`ass` are all non-GPL. So on macOS you get hardware-accelerated encoding in a **pure LGPL build with no x264/x265**:

```
--disable-gpl --disable-nonfree --enable-videotoolbox
```

Ship it as a **Tauri sidecar**: separate-process invocation is aggregation, so no LGPL relinking obligation on the app binary.

**Corollary — do not use the `ffmpeg-next` or `video-rs` crates.** They *link* FFmpeg and drag that obligation straight into the binary. Use **`ffmpeg-sidecar` (MIT)** to drive the CLI. The current Homebrew ffmpeg is a **GPL** build: fine for development, **not shippable**.

**Remotion is disqualified**, despite being the best authoring experience:
- Free only up to **3 employees**; above that **$0.01/render with a $100/mo floor** — metering renders happening on *users'* machines.
- **Mandatory per-render telemetry** sending the licence key and the **end-user's IP**; opt-out only at Enterprise ($500/mo).
- Terms require an "abstraction layer between the end-user and the Remotion Software" — so the moment a user can edit a template, *their* company needs a licence.

For a local-first, privacy-positioned product that is a structural mismatch, not a line item.

Also rejected: Motion Canvas (dormant since Feb 2025; headless rendering open since 2023; exporter pulls GPL-3.0 `ffmpeg-static`), Revideo (0.x, one maintainer, 29k npm downloads/mo vs Remotion's 6.24M), headless Chrome (determinism flags documented as not working on macOS — the only target platform), Cap (AGPLv3).

---

## 4. Recommended stack

1. **Video = templated composition.** Tauri webview canvas driven by an explicit `seek(t)`, encoded with **WebCodecs `VideoEncoder`** + **`mp4-muxer` (MIT)**. Self-built **LGPL FFmpeg** sidecar with `h264_videotoolbox` for muxing, concat and screen-recording ingest, driven from Rust via **`ffmpeg-sidecar` (MIT)**.
2. **Image = local MLX.** Z-Image-Turbo (Apache-2.0) default, FLUX.2-klein-4B alternate, served by **mflux (MIT)** behind a local HTTP wrapper. Keep `sd-server`'s async-job API shape as the interface contract so engines can be swapped later.
3. **Generative video = cloud only, opt-in, never a requirement.**
4. **Screen capture** = `screencapturekit` / `cidre` crates (MIT), native ScreenCaptureKit.

## 5. Kill criteria, cheapest first

**Test A — ~2 hours, 0 GB.** One real "we shipped X" post → one canvas scene with `seek(t)` → 5 captured frames → ffmpeg `-c:v h264_videotoolbox` → 5-second MP4. **Kill: if a hand-built template cannot produce something worth posting, generative video will not rescue it — and if it can, the local-video question is moot.**

**Test B — ~1 hour, 6.5 GB.** `uv tool install mflux`, pull Z-Image-Turbo 4-bit, generate 3 images at 1024×1024, time them. **Kill: >90 s/image on this M4, or output unusable as a launch graphic.**

**Test C — only if A and B pass, ~$5.** Same brief through fal.ai for a 5-second Wan clip; compare against Test A. **Kill: if the templated clip wins — expected — local video is never built.**

**Do not** start by running Wan or LTX locally. LTX will not fit on disk; Wan may take 10+ minutes and may silently emit black frames.

## 6. Blockers before any of this

- **Free disk.** Test B needs 6.5 GB. Clear space first.
- **The local ffmpeg is a GPL Homebrew build.** Building the LGPL/VideoToolbox variant in CI and notarizing it is a prerequisite for shipping; prebuilt LGPL macOS arm64 builds are scarce.
- **Verify early:** whether Tauri's WKWebView exposes `VideoEncoder` identically to Safari 26. This gates recommendation #1.

---

**Confidence:** every licence claim above was read from primary sources (HF `LICENSE` files, `LICENSE.md`, FFmpeg `configure`, `CMakeLists.txt`, CI workflows), as were the machine specs and the sd.cpp CPU-only finding. Base-M4 speed figures are **extrapolated** from M4 Pro/Max data and flagged as such. Whether MLX's diffusion speedup matches its LLM speedup is **assumed, not verified**.
