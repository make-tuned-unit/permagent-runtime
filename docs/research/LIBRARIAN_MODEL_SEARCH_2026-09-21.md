# The next-best Librarian model on the hardware we have

**Date:** 2026-09-21 · **Status:** research; nothing downloaded, installed or run.
**Asked by:** Jesse — "we never have enough memory to use the 27b qwen model so maybe we pause that for now until i
have better hardware and we find the next best open source model for the librarian."
**Coordinated with:** peer session `permagent-runtime-46` (its findings are credited inline).
**Evidence:** `docs/research/librarian-model-evidence/` (partial — see §6).
**Labels:** `[MODEL-CARD]` vendor's own numbers · `[LOCAL]` observed on our machines · `[CODE]` verified in this repo ·
`[RECALLED]` from my own background knowledge, not re-fetched · `[UNVERIFIED]`.

---

## 0. Recommendation

1. **Bench `qwen3.5:4b` against the incumbent `qwen2.5:7b` — on the M4, as a replacement, not an addition.** It is
   **smaller** than what runs today (3.4 GB vs 4.7 GB) and, on vendor numbers, far stronger at exactly what the
   Librarian needs — following a strict format: **IFEval 89.8, MMLU-Pro 79.1** `[MODEL-CARD]`, against roughly
   IFEval ~71 / MMLU-Pro ~56 for Qwen2.5-7B-Instruct `[RECALLED]`. Apache-2.0, stock Ollama, thinking can be switched
   off. On a machine with no free memory, the only upgrade that is safe to try is one that frees RAM.
2. **`qwen3.5:9b` is the quality pick (IFEval 91.5, MMLU-Pro 82.5) — but at 6.6 GB it belongs on the M1**, and the M1
   is not idle (§3). Treat it as the second arm of the bench, run there with the Sidebar session's agreement.
3. **`granite4.2:8b` (5.3 GB) / `:3b` (2.2 GB)** as the third arm: Apache-2.0, dense, positioned for RAG and structured
   JSON. IBM's Granite line has a reputation for faithful extraction `[RECALLED]` — which is the property we cannot
   find a published number for on any candidate — so it earns a slot on reputation, to be settled by our own bench.
4. **Keep the architecture we already have:** Apple's on-device model first, a small Ollama model as fallback. It is
   already wired that way `[CODE]`.
5. **Before any of it: confirm the Librarian is actually running** (§1). On this machine its schedule appears to have
   been disabled since 2026-09-08.

**None of the candidates publishes a faithfulness or hallucination number for its small sizes.** Every figure below is
the vendor's own. The bench in §5 is the decision; this document only chooses what to put into it.

---

## 1. State of play — three things to know first

**(a) The 27B split is paused** (Jesse's call, 2026-09-20). The three `ai.permagent.qwen38-split.*` launchd jobs are
booted out and disabled; plists kept; re-enable recipe in `~/.local/llama/logs/qwen38-split.log`. It had refused every
start slot on 27 nights and had not served since 2026-08-25.

**(b) The config was not consistent with that pause** — found and fixed by the peer session, not by me:
`PERMAGENT_ROLE_MECHANICAL_*` and `PERMAGENT_ROLE_LOCAL_*` in `~/.permagent/config.yaml` still pointed at
`qwen38_split / qwen3.8-27b`. On Jesse's word to that session ("we can use 7b for the librarian") it repointed both to
`ollama / qwen2.5-coder:7b`, backup at `config.yaml.bak-20260921-roles` `[LOCAL]` verified. Related, from the same
session: `WorkflowRole::Mechanical`'s capability floor is byte-identical to `Local`'s (0.75/0.45), so "Mechanical" was
never a more capable fallback.

**(c) The Librarian's nightly schedule looks disabled on this machine.** `~/.permagent/librarian_schedule.json` reads
`"enabled": false, "model": "qwen2.5:7b", "start_time": "02:00", "duration_minutes": 240`, last modified **2026-09-08**
`[LOCAL]`. The last log line I can find with the schedule *enabled* is 2026-09-05. I could not locate the live launchd
daemon's current log to confirm, and I do not know who turned it off or why. **If it is off, no model choice matters
until it is back on** — and memories created since then have had no describe pass at all.

---

## 2. What the job actually is `[CODE]`

- **Describe** (`librarian.rs` `describe_one` ~488): ~700-token prompt → **≤ 150 tokens**, temperature 0.2. The system
  prompt demands **exactly three labelled fields** — `FACTS` (one sentence, < 30 tokens, no speculation), `TERMS`
  (4–10), `CATEGORIES` (2–5).
- **Format strictness is the whole game.** The parser accepts labelled lines, JSON, or fenced JSON, and as a last
  resort salvages a "first usable sentence" — logged as a data-quality loss (≈12 memories went unsearchable on
  2026-08-22 from raw dumps). Ranges are clamped post-hoc because an earlier model ignored them (18–23 items). Two
  retries.
- **Adjudicate** (`librarian_adjudicator.rs`): supersession — reply parsed as `REPLACED <value> <confidence>` /
  `HOLD` / `UNKNOWN`. **Destructive if wrong** (it retires a fact). Default **off**; "shadow" logs only.
- **Atoms** (`librarian_atoms.rs`): consolidate clusters of up to 24 memories into cited atom lines. Default **off**.
- **Backends:** a dedicated OpenAI-compatible endpoint (`PERMAGENT_LIBRARIAN_ENDPOINT`, backend `ollama|llamacpp`) →
  else **Apple on-device first** → else the Ollama pool. **A remote endpoint needs no code change.**
- **Thinking:** the OpenAI-compatible path already sends `chat_template_kwargs: {"enable_thinking": false}`
  (`mesh/pool.rs:515`). *Unchecked:* whether the Ollama-native streaming path sends `think: false` — it matters for any
  Qwen3.x model, which thinks by default. One grep to settle before the bench.
- **Volume:** one call per undescribed memory, queue drained nightly inside a 240-minute window — hundreds of calls.
  Generation speed matters more than prefill.

So the ranking criteria are, in order: **format adherence → faithfulness → extraction quality → conservative
adjudication → tok/s → RAM → runs on stock Ollama / mainline llama.cpp (hard) → licence → a reliable thinking-off
switch.**

---

## 3. Hardware `[LOCAL]`

| | M4 (dev workstation) | M1 (headless) |
|---|---|---|
| RAM / pressure | 16 GiB; ~94 MB free pages; **swap 11.1 / 12.3 GB used** | 16 GiB; **swap 1.8 / 3.0 GB used** — far healthier |
| Disk | 20 GB free, **96 % full** | 29 GB free, 94 % full |
| Ollama | 0.32.9; models: `qwen2.5:7b`, `qwen2.5-coder:7b`, `qwen25-16k` (4.7 GB each), `minicpm5-2b-q8` (2.7 GB) | client **v0.23.2** (old), **daemon not running** |
| llama.cpp | Homebrew mainline b10470 | mainline build from 2026-08-18 at `~/.local/llama/bin/` |

**The M1 is not idle — a correction to what I said earlier.** By standing agreement it runs **shards 11 and 12 of the
Sidebar session's redaction battery** and is the agreed Permagent build host, with rules: pause if M1 memory free
< 20 %, stop under 15 GiB free disk, never signal `redaction_real.py`, and message `sidebar-14` at start and finish.
Moving the Librarian there is feasible and needs no code change — but it is a negotiation with that session, its
Ollama needs upgrading and starting, the port must be bound to the Tailscale interface only, and the machine must stay
awake through the window. **Not a free win.**

So the realistic envelopes: **M4 ≤ ~5 GB, and only by replacing the resident model; M1 ≤ ~8–10 GB, by agreement.**

---

## 4. Candidates

| Model (Ollama tag) | Disk | Licence | Architecture | IFEval | MMLU-Pro | Notes |
|---|---|---|---|---|---|---|
| **`qwen2.5:7b`** — incumbent | 4.7 GB | Apache-2.0 | dense | ~71 `[RECALLED]` | ~56 `[RECALLED]` | two generations old; the parser's salvage paths were written around its habits |
| **`qwen3.5:4b`** | **3.4 GB** | Apache-2.0 | hybrid Gated-DeltaNet (3 linear : 1 full attention) → small KV cache | **89.8** | **79.1** | IFBench 59.2, BFCL-V4 50.3. Vision in the same weights (unused). 256K context. |
| **`qwen3.5:9b`** | 6.6 GB | Apache-2.0 | same hybrid | **91.5** | **82.5** | IFBench 64.5, BFCL-V4 66.1, long-context AA-LCR 63.0. Too big for the M4's envelope. |
| **`granite4.2:8b`** | 5.3 GB | Apache-2.0 | dense (per the Ollama page) | — | — | RAG, tools, structured JSON; `enable_thinking` + `/set nothink`. **No numbers retrievable** (HF card returned 401). |
| **`granite4.2:3b`** | 2.2 GB | Apache-2.0 | dense | — | — | the smallest serious option; a candidate if even 3.4 GB is too much |
| `gemma4:12b` | ~8 GB `[UNVERIFIED]` | Gemma terms (custom) | — | — | — | newest Gemma; smallest size is 12B → M1 only; custom licence |
| `lfm2.5:8b` | — | LFM licence `[UNVERIFIED]` | hybrid | — | — | tuned for fast tool calling, which the Librarian does not do |
| `qwen3:14b` | 9.3 GB | Apache-2.0 | dense | — | — | previous generation; large KV; superseded by `qwen3.5:9b` at two-thirds the size |
| `qwen3.5:27b` / Gemma 26B-A4B / any ~30B-A3B | 15–18 GB | — | — | — | — | **do not fit 16 GB** — the lesson of the split |
| Apple on-device (~3B) | 0 GB to us | Apple | — | — | — | already first choice; ~4k window fits the ~700-token prompt. An independent audit found its *verbalised confidence* below chance — irrelevant to describe, **disqualifying for the adjudicator's `<confidence>`** |
| Bonsai 2 27B | 5.95 GB | Apache-2.0 | ternary | — | — | needs a vendor fork of llama.cpp → fails the hard constraint (see `BONSAI2_27B_REVIEW_2026-09-20.md`) |

All benchmark figures `[MODEL-CARD]` unless marked. **I found no published faithfulness, hallucination or
summarisation-consistency figure for any small candidate.**

### Answers to the four questions

**(a) Is "stay on `qwen2.5:7b`" the honest answer?** It is the safe default and — per the peer session — what Jesse has
already said is acceptable. But the same-vendor successor at **4B** claims +19 points of instruction-following in
**1.3 GB less memory**. If that survives our bench it is strictly better on the axis that costs us data (format drift
→ salvage → unsearchable memories). It is worth one evening to find out.

**(b) Apple first + a better fallback?** Yes, and it is already built. Two cautions: the on-device model must never
supply the adjudicator's confidence number; and we have no measurement of how often the Apple path actually serves vs
falls through — worth logging before tuning the fallback.

**(c) Is moving the run to the M1 the biggest win?** It is the only way to run anything larger than ~5 GB, and the M1
is far less starved (1.8 GB swap vs 11.1 GB). But see §3: it is shared, by agreement, with a long-running battery. The
bigger and cheaper win is **(a)** — a smaller, better model on the M4.

**(d) 14B-class on the M1?** Not worth it. `qwen3.5:9b` at 6.6 GB out-scores the older 14B on the vendor's own tables,
and the hybrid architecture keeps its KV cache small. On a 150-token structured-extraction job the gap between 9B and
14B is unlikely to be visible; the gap between 4B and 9B might be.

---

## 5. Bench plan — for approval

**Instrument.** `~/spectral-local-bench/r40-gate-2026-08-19/gate27b.py` already targets **any** OpenAI-compatible
`/v1/chat/completions` endpoint (model = arg 1, URL = arg 2). Today it measures hall classification accuracy,
self-consistency and relation-triple validity — i.e. the *adjudicator* side. For describe, reuse its harness shape and
swap in `LIBRARIAN_SYSTEM_PROMPT` and the real parser's rules. Ollama serves an OpenAI-compatible endpoint at
`:11434/v1`, so **no new server is needed** for any arm.

**Arms.** `qwen2.5:7b` (incumbent) · `qwen3.5:4b` · `granite4.2:3b` on the M4, one at a time, never resident
together · `qwen3.5:9b` · `granite4.2:8b` on the M1, by agreement with `sidebar-14`.

**Sample.** 100 real memories, stratified by source (chat turn, document, meeting, activity), fixed seed; the same 100
for every arm.

| Measure | How | Go |
|---|---|---|
| **Format adherence** | % parsed by the primary labelled-field path — **no JSON fallback, no salvage, no retry** | ≥ 98 %, and ≥ incumbent |
| **Range compliance** | TERMS 4–10 and CATEGORIES 2–5 **before** clamping | ≥ 95 % |
| **Faithfulness** | every claim in `FACTS` traceable to the source text; hand-checked on all 100 | **zero** invented facts; ≤ incumbent on omissions |
| **Blind A/B** | each arm vs incumbent, unlabelled, pick better / equivalent | wins-or-ties ≥ 70 % |
| **Adjudicator conservatism** | `gate27b.py` as-is, plus 20 pairs where the right answer is `HOLD` | false `REPLACED` = 0 |
| **Speed / memory** | generation tok/s; peak RSS at 1 Hz; wall time for 100 | RSS ≤ incumbent on the M4; whole nightly queue inside 240 min |
| **Thinking leak** | any `<think>` or reasoning preamble in output | 0 |

**Who judges the A/B:** a human, or a frontier model *not* in the running — **never one arm judging another**
(measured elsewhere: the same system scored +0.053 under its own labels and −0.028 under independent ones).

**Cost.** Downloads 3.4 + 2.2 GB on the M4 (20 GB free, 96 % full — delete a duplicate first: `qwen2.5-coder:7b` and
`qwen25-16k` are 4.7 GB each, but **the peer session has just pointed the LOCAL and MECHANICAL roles at
`qwen2.5-coder:7b`, so that one stays**); 6.6 + 5.3 GB on the M1. One evening. No cargo, no daemon change.

**Promotion is one line:** `"model"` in `~/.permagent/librarian_schedule.json`.

---

## 6. What this research could not establish

Two research agents died on usage limits; this was finished with targeted reads. **Not established:** any faithfulness
or hallucination figure for a small candidate; Granite 4.2's benchmark tables (HF returned 401) and whether any 4.2
size is a hybrid-Mamba variant; base-M-series tok/s for any candidate; Gemma 4 12B's disk size and licence detail;
LFM 2.5's licence; whether Ollama 0.32.9 honours thinking-off for `qwen3.5` on the path the Librarian uses; whether the
Librarian is currently running at all (§1c); how often the Apple on-device path serves. The `[RECALLED]` Qwen2.5-7B
figures are from memory and should be re-checked before being quoted anywhere.
