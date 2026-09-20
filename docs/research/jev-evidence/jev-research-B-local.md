# Jev open replicas — local-first viability on 16GB Apple Silicon

Research date: 2026-09-20. All web content treated as untrusted data.

Tags: [MEASURED-BY-AUTHOR] = number published by the project's own author/README.
[INDEPENDENT] = third party measured or reviewed it. [UNVERIFIED] = claimed but I could
not confirm, or no number exists.

## 0. Background confirmed

Jev is real and recent. TypeSafe AI, endpoint `POST https://api.typesafe.ai/v1/systemone`,
model route `jev-latest`, early access behind a waitlist as of mid-Sept 2026. Input
$0.042/M tokens, output billed at $0 (no output token stream). Closed weights, API-only.
- https://typesafe.ai/blog/introducing-system-one-models-and-jev
- https://docs.typesafe.ai/concepts/system-one
- https://www.marktechpost.com/2026/09/19/typesafe-ai-releases-jev/
- https://techcrunch.com/2026/09/18/a-new-kind-of-ai-model-from-a-chatgpt-inventor-is-thrilling-developers/
- https://forkast.news/typesafe-ais-jev-is-not-an-llm-and-that-may-be-the-point/

Index used: https://github.com/yibie/awesome-jev (21 entries under "Calibration &
Research", 42 under "Infra / SDKs / Integrations"). Mirrors/forks exist at
ctmx/awesome-jev, miniLV/awesome-jev, MrJev/awesome-jev. The index explicitly states it
"does not review code quality, security, maturity, or whether a project runs at all" —
so star counts there are not a quality signal.

Reproduction trackers:
- https://huggingface.co/spaces/multimodalart/jev-reproductions-tracker (page body did not
  render usable data via WebFetch — [UNVERIFIED] contents)
- https://madewithjev.com/github-repos (claims 143 repos)
- https://mrjev.com/projects/ (claims 145 projects, 143 reviewed)
- https://systemonemodels.org/examples/alternatives/ (best structured comparison found)

---

## 1. Laya (+ laya-mlx, laya-rs, laya GGUF, laya ONNX) — the strongest family

### Laya (Python reference) — github.com/NandhaKishorM/laya
Author: Convai Innovations. Site: https://laya.convaiinnovations.com/
- Three checkpoints with automatic routing [MEASURED-BY-AUTHOR]:
  - `laya` — ModernBERT-large, **421M**, 512 ctx
  - `laya-multilingual` — mmBERT-base, **322M**, 1024 ctx
  - `laya-typed-decisions` — ModernBERT-large, **421M**, 1024 ctx
  (docs claim RoPE extension up to 8192; default head_max_len 192–256)
- **Encoder, not a decoder LLM.** Non-autoregressive: choice/score/noul are scored in ONE
  encoder forward pass, no token generation at all. This is architecturally the closest
  thing to what Jev actually is, and it is why it is so fast.
- License: **Apache-2.0**, weights included. No non-commercial clause found.
- Latency [MEASURED-BY-AUTHOR], Tesla T4: single question 32.8–39.5 ms; 10 questions
  batched 72.3 ms (7.2 ms/q); 50 questions 337 ms (6.8 ms/q). CPU: 193–464 ms.
- Accuracy/calibration [MEASURED-BY-AUTHOR] on 2,000 typed decisions:
  Laya 0.766 acc / **0.081 ECE** (post-temperature) vs Jev 1.13.0 0.727 acc / 0.144 ECE.
  Site claims ECE 0.081 vs Jev 0.246 and "3x better calibration" — note the two Jev ECE
  figures (0.144 vs 0.246) are inconsistent between README and marketing site. [UNVERIFIED]
  which is right. As-shipped (pre-temperature) ECE was 0.466 — i.e. the raw head is badly
  calibrated and temperature fitting is doing the work.
- API: Python SDK (`Router.predict()`, `agent.predict()`). **Not** `/v1/systemone` shaped.
- pip install laya. Stars ~3.8k, 340 forks, 54 commits.
- Red flag [INDEPENDENT]: Latent Space AINews reported the author was "salty that he did
  not get recognition" and that the RLCD claim is the author's own.
  https://www.latent.space/p/ainews-here-are-6-clones-of-jev-in

### laya-mlx — github.com/mizorewww/laya-mlx
- "Open-weight typed decisions, running natively on Apple Silicon."
- License Apache-2.0. Python 3.11+, macOS 14+, Apple Silicon, MLX 0.32.2. No PyTorch.
- Latency [MEASURED-BY-AUTHOR] on **M3 Max (40 GPU cores, 128 GiB)**, FP16:
  one short question P50 **13.42 ms**, P95 13.92 ms; 50-question throughput 146.8 q/s.
  NOTE: M3 Max/40-core is much faster than an M4 mini (10-core GPU) or M1 mini (8-core).
  Expect meaningfully worse on your boxes — no M4/M1 numbers published. [UNVERIFIED for M4/M1]
- Parity [MEASURED-BY-AUTHOR]: all three checkpoints matched upstream selected answer on
  63/63 validation questions in both FP32 and FP16 (378/378 comparisons). Retains original
  temperature calibration and output schema.
- **Python library + CLI only. No HTTP server.** → needs a Python sidecar for Rust.
- Stars ~1.6k but only **5 commits on main** — very young.
- Third-party writeup: https://aiidelist.com/blog/what-is-laya-mlx

### laya GGUF + `laya` binary (ggmlc) — huggingface.co/mys/laya-typed-decisions-GGUF
**This is the most Rust-friendly artifact in the whole ecosystem.**
- Compiled with **ggmlc**, a compiler that lowers PyTorch/JAX/Flax/Keras to GGML.
- Files: F16 ~811 MB, Q8_0 ~434 MB, UD_Q4_K_M ~404 MB (1D norms/biases kept F32).
- **These are NOT llama.cpp GGUFs. `llama-cli`/llama.cpp will fail to load them.**
  They require the dedicated `laya` binary from ggmlc releases.
- `--device auto` = CUDA or **Metal** if present, else CPU. Apple Silicon Metal supported.
- `laya decide <gguf> --preset invoice --device auto`
- `laya serve` → Decision Studio at `GET /` and **`POST /api/decide`** on port 8080.
  → A single static binary + HTTP = callable from Rust with zero Python. Not
  `/v1/systemone` shaped, so you'd write a thin adapter.
- License: Apache-2.0 upstream weights; ggmlc compiler MIT.
- **No Apple Silicon latency numbers published for the GGUF path.** [UNVERIFIED]
- Sibling: huggingface.co/mys/laya-GGUF (base English checkpoint).

### laya-rs — github.com/apiplant/laya-rs
- "From-scratch Rust reimplementation… built on candle." Implements ModernBERT encoder,
  decision head with temperature calibration, RLCD training loop, Unicode language routing.
- License Apache-2.0. Crate `laya-rs` (docs.rs/laya-rs). CLI binary + Rust library.
- **Backends: CPU default, optional CUDA (+flash-attn). NO Metal, NO Accelerate mentioned.**
  This is the killer caveat — on your Macs it would run on CPU.
- Latency [MEASURED-BY-AUTHOR]: RTX 4090 349 ms (CUDA) / 135 ms (flash-attn); reference
  Python 165 ms; isolated forward pass 60.3 ms vs PyTorch 63.1 ms.
- Weights not bundled; download `convaiinnovations/laya` from HF.
- **Red flags: 2 stars, 1 fork, 15 commits, 0 issues, no version in metadata, explicitly
  "not affiliated with Convai Innovations", no accuracy parity numbers published.**

### laya ONNX — huggingface.co/tozp/laya-onnx
- Single ONNX graph: inputs `input_ids, attention_mask, marker_pos, marker_mask, qtype`;
  outputs `logits, act`. Opset 14 (TorchScript exporter). FP32 and INT8.
- License Apache-2.0. File sizes not stated. [UNVERIFIED]
- No statement about onnxruntime on Apple Silicon or the CoreML EP. [UNVERIFIED]
- But: ONNX + `ort` crate is a well-trodden Rust path, and 421M ModernBERT is small.
  Tokenizer is HF `tokenizers` → also has a Rust crate. This is the second-best Rust path.
- Node/TS ONNX runner also exists: github.com/receptron/laya

---

## 2. von — github.com/wfzyx/von
- **395M**, 1.5 GB, ModernBERT-Large backbone, bidirectional non-autoregressive.
- License **Apache-2.0**.
- Runtimes: Python 3.12+, **and TypeScript/JS (Node & Bun)** "Local In-Process (Sub-18ms)".
  Backends: CUDA, ROCm, **Apple Silicon MPS**, multithreaded CPU.
- Latency [MEASURED-BY-AUTHOR]: MPS/GPU ~18 ms; **CPU ~480 ms**. Hardware not named for
  the MPS figure. [UNVERIFIED which Mac]
- Accuracy [MEASURED-BY-AUTHOR]: Von-1.0 93.5% macro / 93.6% micro vs Jev 1.13 97.2% /
  97.4%. Beats GLiNER2 by +15.0 macro. ViZDoom: Von 9.38 kills vs Jev 5.62 (+66.9%).
- Calibration: post-trained with joint cross-entropy + **Brier score loss**; temperature
  scaling T = 1.1692. ECE "optimized" but **no ECE number published**. [UNVERIFIED]
- **API: "Fully compatible with the TypeSafe `/v1/systemone` specification."** ← the only
  encoder-class model that claims wire compatibility.
- Context length not documented. [UNVERIFIED]
- Training corpus: ~290,000 balanced examples (Ops 25%, Security/DevOps 20%,
  Safety/Moderation 15%, Linguistics 15%, Triage 10%, Adversarial Reasoning 15%).
- 184 stars, 52 commits on master. No ONNX/GGUF/Rust binding found. [UNVERIFIED]

---

## 3. jev-local — github.com/us/jev-local
- Not a trained replica — a **server wrapper** that does prefill+logprob scoring over an
  off-the-shelf Qwen.
- Default `Qwen3.5-9B` (~18 GB weights, needs 24GB+). Light: `Qwen2.5-3B-Instruct`
  (~6 GB, "fits a 16GB Mac").
- Runtime: `uv run uvicorn` + HF transformers (`JEVLOCAL_SCORER=hf`). Python sidecar required.
- API: **`POST /v1/systemone`** with typed noul/choice/score. Correct wire shape.
- Accuracy [MEASURED-BY-AUTHOR]: Qwen3.5-9B **0.916 [0.85, 0.95]** on their decision set.
  The brief's "0.83 overall calibration" figure was **not found on the page I fetched**
  — [UNVERIFIED].
- Latency [MEASURED-BY-AUTHOR], 9B: 0.25 s/question noul, 0.46 s choice, 0.38 s score.
  That is **20–40x slower than Laya** and this is on a GPU box, not a Mac.
- License: **not stated in the README.** ← flag. [UNVERIFIED]
- **3 stars.** Essentially unmaintained/unreviewed.

---

## 4. decider — github.com/Mapika/decider
- Fine-tune of **Qwen3.5-2B** (~1.9B params) + a 35B MoE variant (34.7B total, 3B active).
- License **Apache-2.0**. Weights released on HF.
- **Requires a CUDA GPU.** GH200 bf16: single request ~4 ms, batch of 32 ~74 ms. 35B on a
  B300: ~47 ms/request. **No Apple Silicon support mentioned.** [UNVERIFIED on Mac]
- Accuracy/calibration [MEASURED-BY-AUTHOR]:
  - 2B v10: in-task 0.805 acc / **0.037 ECE**; held-out 0.755 / **0.084 ECE**
  - 35B v1: in-task 0.855 / 0.026; held-out 0.810 / 0.069
  - Browser tasks: 2B 93%, 35B 97.2%
- API: `POST /v1/systemone` in TypeSafe wire format, plus `/decide`.
- 154 stars.
- Red flags (author's own): knowledge-heavy tasks poor at 2B (MMLU only +19 at 35B);
  English-only; calibration measured on public datasets not real traffic; schema cache
  trades 1.5–5 accuracy points for speed; vision variant stuck on v5 weights.

---

## 5. NanoJev — github.com/TianyuCodings/NanoJev
- Base **Qwen3-0.6B**. License **MIT**. Full training pipeline + weights + dataset shipped.
- Training: complete-question cross entropy, mixed-task weighting (Maze 1/3, Snake 1/3,
  Basic 1/6, Predict Position 1/6). Dataset 18,760 questions incl. a calibration split.
- Results [MEASURED-BY-AUTHOR] — note these are **game/agent tasks only**, not general:
  ViZDoom Basic 128/128 vs Jev 56/128; Predict Position 27/128 vs Jev 11/128;
  Maze 4/10 vs Jev 7/10; Snake 8/8 vs Jev 8/8.
- **No calibration metric reported** despite having a calibration split. [UNVERIFIED]
- **No latency numbers. CUDA-oriented** (`serve_decisions.py`). Apple Silicon not mentioned.
- API: `POST /api/evaluate` — **not** `/v1/systemone`.
- 1.4k stars, 39 commits. Doc references an update dated 2026-09-20 (today) → actively worked on.
- The generality is the worry: it is trained on maze/snake/ViZDoom, not on the kind of
  decisions a personal agent runtime makes.

---

## 6. LitJev — github.com/zhengxuyu/litjev
- "Convert a Qwen into a fast decision model with no training."
- Supported: "the full Qwen family… Qwen3.x text and vision checkpoints, any size."
  Default is `Qwen/Qwen3.8-27B` **on an H100 80GB**.
- License: Apache-2.0 code; adapted examples MIT; model weights keep their own licenses.
- API: **matches the Jev schema exactly — `POST /v1/systemone`** with `model`, `state`,
  `questions` → `answers` + `usage`, incl. probabilities and confidence.
- Runtime: `uv` / Python. **No vLLM/llama.cpp/MLX/Ollama integration documented.**
- **The README does NOT explain the decoding mechanism**, only that scores are read
  "directly from the model's output head" without generating answer text. [UNVERIFIED mechanism]
- **No latency, no accuracy, no calibration numbers at all.** Docs reference calibration
  but publish no figures. [UNVERIFIED]
- 28 stars, 20 commits. Immature.

---

## 7. kev — github.com/jaredpalmer/kev
- Base **Qwen3** in 0.6B / 4B / 8B. License Apache-2.0 (code + Qwen3 bases).
- Recipe: 2 epochs, **LoRA rank 16**, cross-entropy. LR 1e-4 (0.6B), 5e-5 (4B/8B).
- **Training time: ~2 h on an M5 for Kev-0.6B.** [MEASURED-BY-AUTHOR]
- Data: 10,000 examples from ten public datasets + 896 generated policy examples +
  1,680 examples from 60 generated rule structures.
- Runtime: PyTorch + LoRA adapters, **MPS on Apple Silicon** (SDPA attention on Apple GPUs,
  pads MPS inputs to 64-token buckets), CUDA elsewhere. Python sidecar required.
- Latency [MEASURED-BY-AUTHOR]: **296 ms median for five questions × three options on an
  M5** (232 tokens). Per-question that's ~59 ms — but M5 ≠ M4 mini / M1 mini.
- Accuracy [MEASURED-BY-AUTHOR]: Kev-4B 0.854/0.856 on trained sources, 0.790/0.806 on
  new sources (dev/test).
- **Calibration red flag, author's own: on new sources Kev-4B assigns ≥0.9 probability to
  a WRONG answer on 8.2% of questions.** That is a serious overconfidence failure for
  anything gating a shell command or a trade.
- API: **`POST /v1/systemone`** with state/model/questions, types noul/choice/score.
- 979 stars. Latent Space describes a Kev-0.5B LoRA on Qwen2.5-0.5B targeting MacBook Pro.

---

## 8. Luce — github.com/scienthoon/luce  (the distillation recipe)
- Base **Qwen3-4B-Base** (also 1.7B-Base, 0.6B-Base). **LoRA r16 + decision head**,
  soft-target cross-entropy + Brier loss.
- Teacher: any OpenAI-compatible API — OpenAI, Anthropic via Vercel AI Gateway,
  **or a local Ollama model**. ← matters: you can distil entirely offline.
- **Data volume: 500–3,000 synthetic labels typical; minimum ~100 REAL labels to fit the
  temperature.** [MEASURED-BY-AUTHOR]
- **Training time: 20–70 min/task on an RTX 4070 SUPER (12 GB); 2–3x faster on a 4090.**
  No Apple Silicon training numbers. [UNVERIFIED on Mac]
- Reported gains [MEASURED-BY-AUTHOR]:
  - support ticket routing **91.1% vs 75.1% zero-shot Jev**
  - phishing detection **97.4% vs 62.6% Jev**
  - organisational-rule priority 74.6%
- Calibration: **ECE 0.003–0.022 in-domain.** Author states plainly: **"Calibration does
  not transfer" to unseen datasets.**
- Inference: one forward pass per option, no text generation at serving time.
  0.25–0.30 s per three-question ticket on a 4070 SUPER; 8 s model load.
- License Apache-2.0. **Only 6 stars** — recipe is good, project is unproven.

## 9. JevForge — github.com/zwliJay/jev-forge
- Base **Qwen3.5-0.8B** (also 0.6B). MIT code; weights+dataset on HF with third-party notices.
- Full loop: synthesize decision data → train calibrated candidate scorers → evaluate →
  serve Jev-compatible inference. Trained on **Mind2Web** (website-disjoint splits).
- **"On an Apple Silicon Mac, one command downloads the 0.8B checkpoint… starts the local
  Jev-compatible API."** [MEASURED-BY-AUTHOR]
- Results [MEASURED-BY-AUTHOR]: test choice top-1 **0.579**; OOD choice top-1 0.637;
  Brier 0.128 (test noul). Those accuracies are low.
- API: REST, state + questions {type, instructions, criteria} → probability distributions.
- **2 stars, 1 commit on main.** Essentially a snapshot drop. Use the recipe, not the repo.

## 10. CUA-S1-FORMS — huggingface.co/cua-ai/cua-s1-forms  (the tiny-specialist proof)
- **706,048 trainable parameters (~706 KB).** Byte-level embedding + 2-layer transformer
  encoder (width 128, 4 heads). "Each option becomes a query against the context tokens."
- License **MIT**. **Safetensors + JSON sidecar; explicitly rejects pickled .pt for security.**
- Scores form-filling decisions: probabilities over document entities / fixed actions
  (fill / check / click / skip) per UI element. All elements scored **in parallel in one batch**.
- Results [MEASURED-BY-AUTHOR]: **99.7% on its test set vs 83.6% for hosted Jev**;
  96% on judgment-requiring decisions; 74% on recognising pre-filled fields.
- Training data: **10,000 synthetic episodes** with synonym and forced-confuser pairs.
- **No latency published** — but at 706K params it is trivially fast and trivially
  portable (a 2-layer encoder is ~200 lines of candle). [UNVERIFIED latency]
- This is the single most important data point for your use case: a 706 KB task-specific
  scorer beat the hosted frontier product by 16 points on that task.

---

## 11. Others surveyed (lower priority)

- **openJev-verdict-2.0** — github.com/Heman10x-NGU/openJev-verdict-2.0. ModernBERT-base,
  **149.6M** (README also says 151M). Apache-2.0. Dual-channel calibration (distribution
  head + confidence head). [MEASURED-BY-AUTHOR] acc **77.10%** (vs Laya 76.60%, Jev 72.70%),
  **Brier 0.0636**, **ECE 0.0144**, option flip rate 4.76% over 2,918 perturbations.
  ~20–25 ms/decision; <35 ms in-browser via **WebGPU under 600 MB**. Trained in 8.8 h on a
  GTX 1660 Ti (6 GB). 176 stars, 27 forks, 25 commits. Red flags: weights are Git LFS
  pointers; two models in one repo; no independent verification.
  ← Best calibration numbers of anything surveyed, and small. Worth a look.
- **mini-jev** — github.com/r-ms/mini-jev. Qwen3-4B-Instruct-2507, ~8.5 GB, MIT.
  CUDA **or MPS (Apple Silicon)**; no MLX. Flat JSON schema, not /v1/systemone.
  4x faster than JSON decoding on short texts. Author states outputs are
  **"not calibrated probabilities; use the gap for abstention."** 33 stars, 6 commits.
- **open-alternative-jev** — github.com/ikermoel/open-alternative-jev. Apache-2.0. Wraps
  any ChatML open-weights LLM. Tested Qwen3.5-4B / Qwen3.6-27B on an H200; CPU works for
  small models; **Apple Silicon not mentioned**. `Decider` class, `decide()`.
  Throughput 49.6 q/s (4B, packed). **Temperature scaling: ECE 5.4% → 2.1% on MMLU with
  one scalar fitted on held-out labels.** 34 stars.
- **jevlike** — github.com/vinnylarouge/jevlike. MIT. Option-as-query attention over
  context tokens; ~40K byte-embedding model per Latent Space. CPU / **Apple MPS** / CUDA.
  98% on synthetic menus; **26% on Wikispeedia with a frozen Qwen2.5-0.5B encoder**.
  4608 menus/sec on CPU. 1.1k stars but **3 commits**. Authors: "a research starter, not
  a copy of Jev."
- **jevbetter** — github.com/olanotolu/jevbetter. MIT. Rival-aware option attention over
  mean-pooled n-gram vectors. [MEASURED-BY-AUTHOR] top-1 **0.916**, top-3 0.999, MRR 0.955,
  **ECE 0.0182** on held-out test. 40 menus/sec on CPU (vs jevlike's 4608). 13 stars, 24 commits.
- **poorjev** — github.com/rupeshpoojary9/poorjev. MIT. One small NLI model (not named),
  one-time ~400 MB download, **CPU, offline**. Primitives Choice/Score/Noul.
  [MEASURED-BY-AUTHOR] **ECE 0.170 → 0.071** via temperature scaling (T=2.71 fitted by
  5-fold CV), accuracy unchanged at 0.781. Adds **conformal thresholding** to turn a risk
  budget into an "I don't know, escalate" signal. Honest README ("slower, honest about it";
  "we report latency, we do not market it"). **2 stars.** ← the calibration methodology here
  is the most rigorous of any repo surveyed, even if the model is weak.
- **SemIf / openjev (TheoLeeCJ)** — Qwen3.5-4B, MIT, RTX 3090 class, **no calibration
  claimed**. Tiny three-class NLI classifier on the last token.
- **minojev** (zeredy879), **jevinci** (achimala), **jevlike-esp32** (david-cermak),
  **advocaat**, **jevkit** (Rust CLI, ariel-frischer — client for hosted Jev, not a model)
  — noted, not evaluated.

## 12. ekzhang/openjev-sglang — the reference for the "no training" trick
https://github.com/ekzhang/openjev-sglang · https://x.com/ekzhang1/status/2100651678110515383
- Qwen3.6-35B-A3B (NVIDIA checkpoint) on SGLang 0.5.19 (Rust frontend), B200 GPU, Modal deploy.
- **Claim: 64 tasks in <1s.** [MEASURED-BY-AUTHOR]
- 221 stars. **No LICENSE detected** per systemonemodels.org.
- Mechanism, verbatim-ish:
  1. Render the native chat template **once**, thinking disabled. Split out a common prefix;
     independently tokenize each question suffix.
  2. Prime SGLang's **radix cache** with a `/generate` call, `max_new_tokens=1`, on the
     shared prefix. This is the "single prefill".
  3. For each question concurrently: send `prefix + question suffix + assistant header +
     "Answer:\n"` with `max_new_tokens=1` and `token_ids_logprob` for every answer label.
  4. Answer tokens are single chars `A`–`Z`, or verified single-token pairs (`AA`, `AB`…)
     for up to 64 options; **all 64 labels are checked against the real tokenizer at
     startup** to guarantee one-token readout. The sampled token itself is discarded.
  5. Stable-softmax renormalise the label logprobs.
     noul → P(yes); choice → argmax + full distribution; score → Σ(level_index × p).
  6. `confidence = 1 - H(p)/log(n_options)`, clamped to [0,1].
- **Explicit honesty from the author: "Probabilities are conditioned on the supplied
  options, depend on prompt and label ordering, and are NOT calibrated estimates of
  correctness."** This is the central caveat for the whole no-training approach.

## 13. decision-head-qwen3.5-4b-rlcd-32k (the "MLP on Qwen 4B" item)
https://huggingface.co/AstroHan/decision-head-qwen3.5-4b-rlcd-32k
- It is a **LoRA, not an MLP**: "4.9M trainable parameters (0.12% of the base), attention
  projections only — **no MLP, no embeddings, no output head**." The brief's framing as
  "MLP on Qwen 4B" is inverted; the notable thing is it deliberately has no MLP.
- Reads probabilities straight out of the **frozen vocabulary logits for single letters A–Z**.
- Base Qwen3.5-4B. Apache-2.0. 32,000 training items, single pass, synthetic + public.
- Cost: 9.0 GPU-hours total on one H200 SXM for three seeds plus all evaluation.
- Findings [MEASURED-BY-AUTHOR]: **95.5% in-distribution but WORSE than the base model
  out-of-distribution** — narrow training data produced *negative* transfer, not neutral.
  Author's framing: coverage predicts generalization in both directions.
- ← Directly relevant warning for distilling your own specialist.

---

## (a) The LitJev / parallel-constrained-decoding trick

Yes, any locally served causal LM can be turned into a Jev-shaped endpoint with **no
training**, provided the server returns logprobs for specified tokens. Mechanism (from
openjev-sglang, which is the only repo that documents it properly — LitJev does not):

1. Build one prompt = shared `state` prefix + per-question suffix.
2. Prefill the shared prefix once; reuse the KV cache across all questions (SGLang radix
   cache; llama.cpp equivalent is slot/prefix reuse; MLX you cache manually).
3. For each question, request exactly one token and read the logprobs of the candidate
   label tokens only (`A`…`Z`, verified single-token at startup against the tokenizer).
4. Softmax over just those label logits.
   noul → P(yes); choice → distribution; score → expectation Σ(i · p_i).

Per-runtime feasibility for you:
- **llama.cpp / llama-server**: yes. `/completion` with `n_probs` / `logprobs` and
  `n_predict: 1`, plus `cache_prompt: true` for prefix reuse. Callable from Rust over HTTP,
  or in-process via the `llama-cpp-2` crate. **No Python.** ← best no-training path for you.
- **Ollama**: historically **does not expose logprobs** on `/api/generate`. [UNVERIFIED as
  of 2026-09] — I did not find a confirming source either way in this session. If it still
  doesn't, Ollama can only give you hard choices, and you should point at llama-server
  (which Ollama bundles) directly.
- **MLX**: yes, trivially in Python (`mlx_lm` gives you the logits tensor), but that is a
  Python sidecar. `mlx-rs` exists but you'd hand-roll the model.
- **vLLM/SGLang**: yes, `logprobs` + `allowed_token_ids`. Not relevant to a 16 GB Mac.

**Calibration consequences.** The probabilities are *not* calibrated. Documented failure
modes: (i) they are conditioned on the option set you supplied; (ii) they are sensitive to
prompt wording and **label ordering** (openjev-sglang says so explicitly; openJev-verdict
measures a 4.76% option-flip rate under perturbation); (iii) instruction-tuned models are
systematically overconfident — Laya's own raw head shipped at ECE 0.466 before fitting.

**Fixes people actually use, in order of prevalence:**
- **Temperature scaling** — one scalar on the logits, fitted on a held-out labelled set.
  Universal. Evidence: Laya 0.466 → 0.081 ECE; poorjev 0.170 → 0.071 (T=2.71, 5-fold CV);
  open-alternative-jev 5.4% → 2.1% on MMLU; von ships T = 1.1692.
- **Proper-scoring-rule training** where you do train: von uses joint cross-entropy +
  **Brier**; Luce uses soft-target CE + Brier. This is what RLCD is approximating.
- **Conformal thresholding / abstention** — poorjev turns a target risk budget into an
  "I don't know, escalate" signal. For a personal agent gating shell commands, this is the
  right primitive, more so than a better point estimate.
- **Isotonic / Platt**: Platt is the 2-parameter generalisation of temperature scaling and
  is standard for binary (noul); isotonic is non-parametric and needs more data (rule of
  thumb ~1000+ labels) or it overfits. **I found no Jev-ecosystem repo that reports
  isotonic or Platt results** — temperature scaling dominates. [UNVERIFIED that anyone
  in this ecosystem uses isotonic]
- Debiasing tricks not found documented here but standard: average over label permutations,
  and subtract a content-free-baseline calibration (null-prompt) from the logits.

**Crucial constraint: calibration is per-task.** Luce's author states outright
"calibration does not transfer." So you fit one temperature per decision type in your
runtime, from ~100 real labelled examples each (Luce's stated minimum).

## (b) Apple Foundation Models as a Choice/Noul primitive

**Short answer: it gives you hard choices for free, but NOT probabilities, and its
self-reported confidence is worse than a coin flip. It cannot be a calibrated Noul/Choice
primitive.**

- `@Generable` on a Swift enum gives compile-time schema + **constrained decoding**, so the
  output is guaranteed to be one of your cases. That is a genuine zero-cost, on-device,
  private, hard-choice primitive. `@Guide` adds constraints/ranges/descriptions.
  https://developer.apple.com/videos/play/wwdc2025/301/
- **No logprob API.** I checked the WWDC26 "What's new in the Foundation Models framework"
  session (developer.apple.com/videos/play/wwdc2026/241/): the new surface is
  `model.contextSize`, `model.tokenCount(for:)`, image attachments,
  `PrivateCloudComputeLanguageModel(reasoningLevel:)`, the `LanguageModel` protocol for
  third-party providers, `DynamicProfile`, system tools (BarcodeReaderTool, OCRTool,
  Spotlight), an evaluations framework, an `fm` CLI for macOS 27, a Python SDK, and the
  framework going open source. **The only numeric feedback exposed is token counting**
  (`response.usage.input.totalTokenCount`, `.cachedTokenCount`,
  `.output.reasoningTokenCount`). No logprobs, no per-option probability, no confidence.
- **Independent audit** (beri.net, n=2,900 factual QA + 110 false-premise + 310
  summarization, against Gemma-3-4B-it / Llama-3.2-3B / Ministral-3B) [INDEPENDENT]:
  - "There is no logprob API."
  - Verbalized confidence **AUROC 0.47 (CI 0.456–0.485)** — statistically **below chance**.
    Mean stated confidence 98.8% almost regardless of correctness.
  - **ECE 70.1**, worst of the four models tested.
  - Workaround that works: **k=5 self-consistency**, abstain on disagreement →
    confident confabulation 75.0% → 2.6%; selective accuracy on factual QA 42.7% → 82.8%;
    but **coverage collapses to 28.2%** (answers 28 of 100). Single operating point, not a curve.
  https://www.beri.net/article/apple-on-device-foundation-model-audit-confabulation-consistency-sampling
- Interesting angle from WWDC26: the new `LanguageModel` / `LanguageModelExecutor` protocol
  lets you plug **your own** model in behind the FM API. So you could expose Laya or a
  llama.cpp endpoint through FoundationModels to your Swift iOS/watch apps and keep one
  call site. **I found no write-up of anyone doing Jev-shaped decisions this way.** [UNVERIFIED]
- **No write-ups found of anyone using FM as a Jev substitute.** The @Generable-enum-as-
  classifier pattern is widely written up; none of those posts recover probabilities.

Practical verdict: use `@Generable` enums for cheap, private, non-safety-critical routing
(which of my 6 intents is this?), where being wrong is recoverable. Do not use it where you
need a probability to threshold on — memory retention scoring, shell-command gating,
trading. For those, a 400 MB Laya beats it and actually gives you a number.

## (c) Distilling a domain specialist from your own logged decisions

Converged recipe across Luce, JevForge, kev, CUA-S1 and decision-head-qwen3.5-4b:

1. **Log**: capture (state, question, options, chosen option, outcome) from your runtime.
2. **Amplify with a teacher**: a big model labels your logged states, and you keep its
   **soft** distribution, not just the argmax. Luce accepts any OpenAI-compatible teacher
   including a **local Ollama model** — so this can be done with zero egress. Synthesise
   confusers deliberately: CUA-S1 used "forced confuser pairs" and synonym swaps.
3. **Volume** [MEASURED-BY-AUTHOR]: Luce says **500–3,000 synthetic labels per task** is
   typical, plus a **minimum ~100 real labels** just to fit the temperature. kev used
   10,000 public + 896 policy + 1,680 rule-structure examples. CUA-S1 used 10,000 synthetic
   episodes. decision-head used 32,000 items. So: **low thousands per decision type.**
4. **Train**: LoRA r16 on attention projections + a small decision head; loss = soft-target
   cross-entropy **plus Brier** (the proper scoring rule is what buys you calibration).
   Freeze the backbone. decision-head-qwen3.5-4b trained only 4.9M params (0.12%).
5. **Fit temperature** on held-out real labels, per task.
6. **Serve**: one forward pass per option, no text generation.

**Time/hardware** — and here is the gap for you: **nobody published Mac training times
except kev.** kev: **~2 h on an M5 for the 0.6B**. Luce: 20–70 min/task on an RTX 4070
SUPER 12 GB, 2–3x faster on a 4090, **no Mac figure**. openJev-verdict-2.0: 8.8 h on a GTX
1660 Ti 6 GB for a 150M ModernBERT. decision-head: 9.0 H200-hours for three seeds + all
eval. On a 16 GB M4 expect **hours, not minutes**, for a 0.6B LoRA, and consider training
the 150M-class encoder instead, which is in the same ballpark as a 1660 Ti.

**Reported gains** [MEASURED-BY-AUTHOR]:
- Luce: support ticket routing **91.1% vs 75.1% zero-shot Jev**; phishing **97.4% vs 62.6%**.
  ECE 0.003–0.022 in-domain.
- CUA-S1-FORMS, **706K params**: **99.7% vs hosted Jev's 83.6%** on its task.
- Bespoke Nimble (LoRA on Qwen3.5-9B) [INDEPENDENT, via Latent Space]: base Qwen 66% → 90%,
  vs 93% for Jev, 100 ms on an H100.
- kev-4B: 0.854 in-domain → 0.790 out-of-domain.

**The two warnings, both from the authors themselves:**
- Luce: **"Calibration does not transfer."** Refit temperature per domain.
- decision-head-qwen3.5-4b: narrow training data made the model **worse than its own base
  model out-of-distribution** — negative transfer, not neutral. So keep the specialist
  narrow *and* route to it narrowly; don't let it answer questions outside its coverage.
- kev: on new sources it puts ≥0.9 on a wrong answer **8.2%** of the time.

---

## Sources

https://github.com/yibie/awesome-jev
https://typesafe.ai/blog/introducing-system-one-models-and-jev
https://docs.typesafe.ai/concepts/system-one
https://www.marktechpost.com/2026/09/19/typesafe-ai-releases-jev/
https://techcrunch.com/2026/09/18/a-new-kind-of-ai-model-from-a-chatgpt-inventor-is-thrilling-developers/
https://forkast.news/typesafe-ais-jev-is-not-an-llm-and-that-may-be-the-point/
https://www.datacamp.com/blog/system-one-models-jev
https://www.langchain.com/blog/building-a-harness-with-jev
https://systemonemodels.org/examples/alternatives/
https://www.latent.space/p/ainews-here-are-6-clones-of-jev-in
https://huggingface.co/spaces/multimodalart/jev-reproductions-tracker
https://madewithjev.com/github-repos
https://mrjev.com/projects/
https://github.com/NandhaKishorM/laya
https://laya.convaiinnovations.com/
https://github.com/mizorewww/laya-mlx
https://aiidelist.com/blog/what-is-laya-mlx
https://huggingface.co/mys/laya-typed-decisions-GGUF
https://huggingface.co/mys/laya-GGUF
https://github.com/apiplant/laya-rs
https://docs.rs/laya-rs/latest/laya/
https://huggingface.co/tozp/laya-onnx
https://github.com/receptron/laya
https://github.com/wfzyx/von
https://github.com/us/jev-local
https://github.com/Mapika/decider
https://github.com/TianyuCodings/NanoJev
https://github.com/zhengxuyu/litjev
https://github.com/jaredpalmer/kev
https://github.com/scienthoon/luce
https://github.com/zwliJay/jev-forge
https://huggingface.co/cua-ai/cua-s1-forms
https://github.com/Heman10x-NGU/openJev-verdict-2.0
https://github.com/r-ms/mini-jev
https://github.com/ikermoel/open-alternative-jev
https://github.com/vinnylarouge/jevlike
https://github.com/olanotolu/jevbetter
https://github.com/rupeshpoojary9/poorjev
https://github.com/ekzhang/openjev-sglang
https://x.com/ekzhang1/status/2100651678110515383
https://huggingface.co/AstroHan/decision-head-qwen3.5-4b-rlcd-32k
https://developer.apple.com/videos/play/wwdc2025/301/
https://developer.apple.com/videos/play/wwdc2026/241/
https://developer.apple.com/videos/play/wwdc2026/339/
https://www.beri.net/article/apple-on-device-foundation-model-audit-confabulation-consistency-sampling
https://www.createwithswift.com/exploring-the-foundation-models-framework/
