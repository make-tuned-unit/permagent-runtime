# Laya as the on-device Jev — tested 2026-09-22

**Ask (2026-09-22):** analyse Laya as a local open-source alternative to Jev, run it on this machine, test it on the
Jev workflows, and find what else an on-device decision model unlocks now that nothing leaves the device.

**Verdict in one paragraph.** Laya runs on this Mac (M4, 16 GB) at 60–135 ms per multi-question decision on Metal
through its official Python package, and it speaks TypeSafe's exact `/v1/systemone` contract, so Permagent's
`systemone` judge provider talks to it unchanged on loopback. On the real captured Jev request it agrees with Jev on
every answer. **On Permagent's own workflows, zero-shot, it does not:** it got the memory-classification and
recall-relevance questions wrong where Jev got them right, and it called an obvious prompt injection "safe to act
on" at 0.65. Its own model card says so ("base checkpoints are near chance on typed-decisions zero-shot"). The
model is Apache-2.0 and trainable with a published recipe, and Permagent is about to journal every decision (B2b-i),
so the honest plan is **Laya as the always-on shadow backend that learns from the journal**, with Jev and the local
LLM as labelers, promoted per site when agreement clears a measured bar — not Laya as a drop-in for Jev today.

## 1. What Laya is (from the model card and repo; verified)

| | |
|---|---|
| Publisher / license | Convai Innovations · **Apache 2.0** · `convaiinnovations/laya` on Hugging Face; code `NandhaKishorM/laya` (PyPI `laya` 0.3.5) |
| Architecture | ModernBERT-large encoder (395 M) + decision head = **421 M params**; 512-token context (English); one encoder pass, no generated tokens |
| Training | RLCD — "the policy reports a distribution; exploration adds zero-mean Gaussian noise to the logits; the reward is a strictly proper scoring rule" |
| Question types | `choice`, `score`, `noul` — the same three as Jev |
| Variants | `laya` (English), `laya-multilingual` (mmBERT-base, 322 M, 1,024 ctx, 100+ languages), `laya-typed-decisions` (four specialist workflows) |
| Card's own numbers vs Jev | typed-decisions 0.766 vs 0.727 · AG News 0.950 vs 0.910 · ECE **0.081 vs 0.246** · p50 latency 32.8 ms vs 236–276 ms; weak at >20 options (Banking77 0.425 vs 0.870) and on ordinal `score` (0.372 SST-5) |
| Runtimes | Python (`laya.load(...).predict(state, questions)`, torch/MPS) · GGUF via `ggmlc`'s `laya` CLI (`mys/laya-GGUF`, F16 807 MB / Q8 431 MB / Q4 401 MB) · ONNX (`tozp/laya-onnx`) · CoreML incl. an ANE build (`aac6fef/laya-multilingual-coreml-ane`) |
| `laya serve` API | `POST /v1/systemone` (TypeSafe body and response, `usage.input_tokens`), `/v1/decide`, `/v1/decide/batch` (≤256 states), `GET /v1/models`, `/health`; optional bearer auth via `LAYA_API_KEY` |

**Name check:** there is no model called "Laya Core LM" on Hugging Face; the closest candidates are the official
`convaiinnovations/laya` (used here) and community conversions of it. The "core" in the ask most likely refers to the
CoreML builds (`aac6fef/laya-coreml`, `FluidInference/laya-coreml`).

## 2. Running it here

| Path | Result |
|---|---|
| **ggmlc Metal binary** (`laya-macos-arm64-metal.tar.gz`, v0.9.1 of 2026-09-21 and v0.9.0 of 09-20) | Loads the GGUF on Metal, then **crashes on every decision** — `libc++abi: terminating due to uncaught exception of type std::regex_error: The expression contained an invalid escaped character, or a trailing escape.` Same in `serve` (every POST → 500) and `decide`. Fails *after* load, so it looks like a model problem and is a packaging one (a libc++ `std::regex` incompatibility, most likely the tokenizer's pre-tokenizer pattern). Unusable on macOS as shipped; worth an upstream issue. |
| **Official Python package** (`uv venv` + `laya`, torch 2.14, MPS) | Works. 843 MB weights + 703 MB venv under `~/.permagent/models/laya/`. Cold load 62 s (first MPS compile), first predict 2.4 s, then **61–134 ms median** per multi-question call. |

The evaluation below used the Python path. **Astra's ruling (2026-09-22):** an SSH tunnel does *not* establish locality —
loopback transport says nothing about where private data is processed — so a Laya server on the M1 needs explicit
endpoint/host trust in the decision module, not a tunnel that makes it look like `127.0.0.1`. Always-on Laya,
fine-tuning and automatic promotion are deferred; one bounded C1 shadow experiment is the funded step, and
"agreement with another model is not ground truth.

## 3. Head to head on the real Jev fixture (the captured 2026-09-21 request)

| Question | Jev | Laya |
|---|---|---|
| Urgent? (`noul`) | 0.95 | 0.78 |
| Department (`choice`) | billing 0.88 | billing 0.84 |
| Frustration (`score`) | "Frustrated" 0.96 | "Frustrated" 0.85 |

Same answers, softer probabilities (Laya's calibration is the point of RLCD). This is Laya's home distribution.

## 4. Head to head on Permagent's workflows (synthetic public inputs; same request to both)

Ground truth is what a careful person would say. ✔ right · ✘ wrong · ~ arguable.

| Workflow · question | Jev | Laya | Who is right |
|---|---|---|---|
| **Watcher pick** — which of 4 candidates deserves a nudge (guide due tomorrow is #2) | **#2 0.75** | **none 0.43**, #2 0.12 | Jev ✔ · Laya ✘ |
| · is the news item about their work (name coincidence) | 0.27 | 0.08 | both ✔ |
| **Memory classification** — kind of "switched from YNAB…, don't suggest YNAB again" | **preference 0.89** | event 0.39 | Jev ✔ · Laya ✘ |
| · durable in six months? | 0.54 | 0.07 | Jev ~ · Laya ✘ |
| · supersedes a prior state? | 0.84 | 0.04 | Jev ✔ · Laya ✘ |
| · sensitivity | confidential 0.62 | personal 0.48 | both ~ |
| **Recall relevance** — memory bears on "which budgeting tool…"? | 0.95 | **0.18** | Jev ✔ · Laya ✘ |
| · usefulness | decisive 0.74 | decisive 0.63 | both ✔ (Laya internally inconsistent with the line above) |
| **Ingest triage** — Dependabot mail needs a full model read? | 0.49 | 0.13 | Laya ✔ (rules can file it) · Jev ~ |
| · category | automated_notice 1.00 | automated_notice 0.63 | both ✔ |
| · urgency | none 0.82 | **today 0.67** | Jev ✔ · Laya ✘ |
| **Injection guard** — content tries to instruct the assistant? | 0.97 | 0.92 | both ✔ |
| · safe to act on it? | **0.02** | **0.65** | Jev ✔ · Laya ✘ — the one that matters |
| **20-option choice** with one real match | opt7 1.00 | opt7 0.9998 | both ✔ (distractors were empty; not a real Banking77-style test) |
| Latency per call | 350–550 ms (network) | 61–134 ms (on device) | Laya |

**Reading:** Laya is right when the answer is on the surface of the text (a bot address, an injection phrase, a
lexical match) and wrong when it needs the *meaning* of a rule ("don't suggest X again" is a preference and a
supersession), the relationship between two texts (recall relevance), or a judgement against a norm (urgency,
safe-to-act). Those are exactly Permagent's high-value questions. Its card predicted this. Jev's answers on these
six were the ones a person would give.

## 5. What this means for the plan

1. **Do not route Permagent's judge questions to Laya zero-shot.** Hosted Jev stays the judge for **public** states
   (the only states it may see), and the local LLM stays the labeler for private ones. The decision module's
   fail-awake and never-over-explicit-state rules stand.
2. **Laya becomes the shadow backend, immediately.** It is free, on device, 100 ms, and speaks the contract. Wire
   it as the `systemone` provider on loopback and run every C1-class site in shadow through `decide_journaled`
   (B2b-i): the journal records Laya's distribution beside the acting decision (the Watcher model's pick, the
   ingest rule's outcome, later the user's action). That produces the labelled dataset for free, from real traffic,
   with **nothing leaving the device** — the property that makes this possible at all.
3. **Fine-tune Laya on the journal** ("learn from outcomes", frontier idea 5). Laya is Apache-2.0 with a published
   RLCD recipe and a per-(type, option-count) temperature calibration step that its card says moves ECE 0.466 → 0.081.
   A per-site checkpoint trained on a few thousand journaled decisions is the realistic route to the >85 % agreement
   bar the Council set for promotion — measured per site, never asserted. Training runs on the M1 overnight.
4. **Promote per site, tighten-only.** A site moves from shadow to acting only where Laya's agreement with the
   labeler clears the bar, and only in the direction that cannot lose work (skip a nudge, file a bot mail, flag an
   injection) — never to allow.

## 6. What an on-device decision model unlocks (candidates, each needs its own node and journal evidence)

All of these are **Private** states and were off the table for hosted Jev by the locality rule. They are on the table
for Laya because it never leaves the device. Ordered by how much the shadow journal already tells us.

| Use | Question shape | Why it fits a decision model | Status |
|---|---|---|---|
| **Memory classification at ingest** (kind · durable · supersedes-prior · sensitivity) | 4 questions per memory | Feeds the Librarian's fields and the stale-fact flag without a generation pass | Laya wrong zero-shot on 3 of 4 → shadow + fine-tune first |
| **Spectral recall re-ranking** (does candidate memory bear on the query; usefulness) | 2 per candidate, batched (`/v1/decide/batch`, ≤256 states) | 100 ms for a page of candidates; the current recall has no second-stage judge | wrong zero-shot on relevance → shadow; the recall log is the labeler |
| **Injection / tool-result guard** (instructs rather than informs) | 1 `noul` per tool result | Right zero-shot (0.92); the "safe to act" question is the wrong one to ask a model — code decides that | shadow now; tighten-only (flag, never clear) |
| **Ingest triage** (needs an LLM pass; category) | 2–3 per item | Right on the surface questions; wrong on urgency | shadow now; category only |
| **Watcher candidate scoring (C1)** | 1 choice over ≤20 candidates + 1 noul | Council's chosen first site | shadow now; the model pick is the label |
| **Adversary inspector second opinion** | 1 `noul` per reviewed tool call | Cheap disagreement signal beside the LLM verdict; never the verdict | after the guard journal exists |
| **Tool-approval hold vs ask** | `choice` {execute, ask, deny} | Only ever tightens (hold when the LLM says allow) | after injection-guard evidence |
| **Voice end-of-turn / barge intent** (V-programme) | `noul` on a transcript fragment | 60 ms fits the latency budget where an LLM cannot | design only |
| **Decision Inbox pre-sort** (tier, kind sanity, "needs a human read") | 2–3 per decision | The review view (D1) can show "the judge would…" beside each card | after D1 |

## 7. Files

- `~/.permagent/models/laya/` — ggmlc binaries (crashing), `laya_english_q8_0.gguf`, `venv/`, `smoke.py`,
  `workflows.py`, `workflow_requests.json`, `jev_responses*.json` (synthetic inputs only).
- Hugging Face cache — `convaiinnovations/laya` weights (843 MB).
- No memory, mail or private state was sent to Jev in this test; every state above was written for the test.
