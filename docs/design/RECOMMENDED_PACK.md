# The Recommended Pack — one tested-and-honest per-role model config

**Date:** 2026-09-02 · **Status:** recommendation (research + evidence lane; no code changed) ·
**Author's rule of the road:** every number below is tagged with how we know it —
**`[MEASURED-here]`** (run on our data, our harness, our hardware),
**`[published]`** (someone else's benchmark, with its URL and read-date),
or **`[reasoned]`** (an architectural inference we have not measured). Nothing is called
"tested" unless a `[MEASURED-here]` row backs it. Where the evidence is a pilot at n≤10, it
says **PILOT** and is treated as a hypothesis, not a result.

---

## 0. TL;DR — the pack

A "pack" here is **one provider+model per role** for the three roles that have genuinely
different shapes: **voice** (first spoken syllable fast), **chat** (first *written* word fast
without losing answer quality), **coding/harness** (highest pass rate per dollar; latency
barely matters over a ten-minute loop). Roles and their config keys are defined in
`crates/goose/src/config/model_roles.rs:73-97` (chat/harness) and
`crates/goose/src/config/voice_model.rs:36-39` (voice).

| Role | **Primary (recommended default)** | Evidence | ~$/unit | **Local-first variant** | **Max-quality variant** |
|---|---|---|---|---|---|
| **Voice** | `custom_deepseek` / `deepseek-chat` | `[MEASURED-here]` | **$0.0018/turn** | `apple_fm` / `apple-on-device` *(social turns only)* | `anthropic` / `claude-haiku-4-5-20251001` |
| **Chat** | `custom_deepseek` / `deepseek-chat` | `[MEASURED-here]` | **~$0.003–0.03/turn** | `apple_fm` (short) → `ollama` / `qwen3` (M4) | `anthropic` / `claude-sonnet-5` |
| **Coding / harness** | `openai` / `gpt-5.4-mini` | `[MEASURED-here]` **PILOT** + projection | **~$0.18/task** | `ollama` / `qwen3-coder:30b` (M4, 16 GB) | `anthropic` / `claude-opus-4.8` (hard) · `claude-sonnet-5` (value-max) |

**The single most important honest caveat:** the Primary column **equals what ships today**
— so "recommended" and "current default" are the same config. That is a real finding, not a
dodge: the defaults are already the best-fit, cost-gated picks the measurements produced. What
is *not* yet true is the word "tested" on the **harness** row — see §4. The voice and chat
Primary rows are genuinely measured; the harness Primary rests on a cache-corrected
**projection** from a 2-of-3-task tie, never a head-to-head coding-suite run.

---

## 1. What the app already knows (inventory)

### 1.1 Current per-role defaults (where each is set)

| Role | Current default | Provider id | Set at | Source tier |
|---|---|---|---|---|
| Chat | `deepseek-chat` | `custom_deepseek` | `model_roles.rs:130-141` (`DEFAULT_CHAT_*`) | `[MEASURED-here]` |
| Harness | `gpt-5.4-mini` | `openai` | `model_roles.rs:145-155` (`DEFAULT_HARNESS_*`) | `[MEASURED-here]` PILOT + projection |
| Voice | `deepseek-chat` | `custom_deepseek` | `voice_model.rs:52-77` (`DEFAULT_VOICE_*`) | `[MEASURED-here]` |

**Precedence** (`model_roles.rs:23-67`): CLI `--provider/--model` > resumed session's saved
model > recipe `settings:` > `<role>_provider`/`<role>_model` > `GOOSE_PROVIDER`/`GOOSE_MODEL`
(the "session model") > the measured default. Voice deliberately puts its default **above**
`GOOSE_MODEL` (`voice_model.rs:15-28`) because a reasoning model on a spoken turn is a
10-second silence; chat/harness put it **below** the session model, because a user who set
`GOOSE_MODEL` made a choice a benchmark should not silently overrule.

### 1.2 The defaults diverge from the bench docs' stated winners — and why (honesty)

The three research notes each name **Claude Haiku 4.5** as their winner. The shipped defaults
are **not** Haiku. This is deliberate and documented in code:

- **Voice** — `VOICE_MODEL_BENCH_2026-08-25.md` recommends `anthropic/claude-haiku-4-5-20251001`
  (warm p90 **1070 ms**, **0** silent turns in 40). The shipped code (`voice_model.rs:56-71`)
  flips it back to `deepseek-chat`: "Haiku … costs **4.1× per turn** — past the ~2× gate the
  same bench originally held. … This constant restores the gate the measurement was held to."
- **Chat** — `MODEL_DEFAULTS_BENCH` part B names Haiku first on quality (**2.07**) and
  cheapest by 3.3×, but Haiku's p90 is **7528 ms**; `deepseek-chat` is the only candidate under
  the original **p90 ≤ 2.5 s** bar (median **1.83 s**, p90 **2.06 s**) — see `model_roles.rs:132-141`.
- **Harness** — `MODEL_DEFAULTS_BENCH` part A names Haiku ("2.8× cheaper, 12× faster than
  incumbent GLM-5.3"), but adds a **provisional** note: cache-corrected, `deepseek` ($0.17) and
  `gpt-5.4-mini` ($0.18) project cheaper. The shipped default is `gpt-5.4-mini`
  (`model_roles.rs:145-155`): "near-cheapest after the [cache] fix [#1122], a different family
  from chat/voice, and the id the harness already knows how to pin."

**Read this way:** the defaults are the **cost-gated re-reads** of the benches, consistent with
the routing principle (best-fit, *very* cost-conscious — not cheapest-first, and not
quality-at-any-price). The recommendation below keeps that posture and adds the two variants
the current single-default config does not express.

### 1.3 Providers wired (relevant to the pack)

- `anthropic`, `openai`, `google`, `deepseek`/`custom_deepseek`, `zai`, `xai`, `moonshot`,
  `minimax` — cloud, all wired (`crates/goose/src/providers/`).
- **`apple_fm`** — Apple **on-device Foundation Models**; inference on this Mac, prompt never
  sent off-device (`apple_fm/mod.rs:1,13-27`). Single model id **`apple-on-device`**
  (`mod.rs:92`), context window **4096 tokens on macOS 26.2**
  (`apple_fm/sidecar.rs:352`), and it **does not accept tool definitions**
  (`mod.rs:199-204`). Honest consequence: it can serve a short, tool-free reply (a social voice
  sentence, a one-line chat) but **cannot** run the harness's 124 tool schemas or a long repo
  map. "Local" is claimed only for inference locality; `PrivateCloudCompute` is *cloud* and is
  deliberately not wired here (`mod.rs:29-31`).
- **`ollama`** — localhost:11434, `OLLAMA_DEFAULT_MODEL = "qwen3"`; known models include
  `qwen3-coder:30b` and `qwen3-coder:480b-cloud` (`ollama.rs:28-36`).

### 1.4 Pricing the catalog carries (`canonical/data/canonical_models.json`, $/Mtok)

| model | in | out | cache-read |
|---|---|---|---|
| `deepseek/deepseek-chat` | 0.28 | 0.42 | 0.028 |
| `openai/gpt-5.4-mini` | 0.75 | 4.5 | 0.075 |
| `openai/gpt-5.4` | 2.5 | 15.0 | 0.25 |
| `openai/gpt-5.4-nano` | 0.2 | 1.25 | 0.02 |
| `anthropic/claude-haiku-4.5` | 1.0 | 5.0 | 0.1 |
| `anthropic/claude-sonnet-5` | 3.0 | 15.0 | 0.3 |
| `anthropic/claude-opus-4.8` | 5.0 | 25.0 | 0.5 |
| `zai/glm-5.3` (incumbent session model) | 1.4 | 4.4 | 0.26 |
| `moonshot/kimi-k2.6` | 0.95 | 4.0 | 0.16 |
| `minimax/MiniMax-M2.7` | 0.3 | 1.2 | 0.06 |
| `google/gemini-2.5-flash-lite` | 0.1 | 0.4 | 0.025 |
| `apple_fm/apple-on-device`, `ollama/qwen3*` | **0** | **0** | — |

Catalog-staleness note (honesty): the JSON tops out at `claude-opus-4.8` /
`gpt-5.4-*`, but the **live cost ledger already bills `openai/gpt-5.6-terra`** (see §1.5) and a
public tracker lists a still-higher tier (`[published]`, artificialanalysis.ai, read
2026-09-02: "Claude Opus 5 / Fable 5.1" as highest-intelligence, "Gemini 2.5 Flash-Lite
non-reasoning" lowest latency at **0.29 s**). Any "max-quality" claim should be re-pinned to
the live frontier at validation time, not frozen to the catalog.

### 1.5 Real measurements the repo already holds

**(a) Live production cost ledger** — `~/.permagent/spectral/permagent.db`, table `cost_ledger`
(schema `session/spectral_schema.rs:2683-2707`, written by
`session_manager.rs:1720-1754`). Aggregated 2026-09-02, real usage on this machine:

| provider / model | calls | total $ | ~$/call |
|---|---|---|---|
| `anthropic/claude-haiku-4-5-20251001` | 2401 | $69.21 | $0.029 |
| `custom_deepseek/deepseek-v4-flash` | 1083 | $8.50 | $0.008 |
| `moonshot/kimi-k2.6` | 434 | $19.04 | $0.044 |
| `minimax/MiniMax-M2.7` | 229 | $1.98 | $0.009 |
| `zai/glm-5.3` | 131 | $13.31 | $0.102 |
| `openai/gpt-5.6-terra` | 62 | $6.83 | $0.110 |
| `anthropic/claude-sonnet-5` | 34 | $2.54 | $0.075 |
| **`ollama/qwen3-coder:30b`** | **15** | **$0.00** | **$0 (local)** |

Two facts fall out: **`custom_deepseek` bills the alias `deepseek-v4-flash`, not
`deepseek-chat`** (also noted in the harness pilot), and **local coding already runs at $0** on
this hardware (`ollama/qwen3-coder:30b`, 15 real calls).

**(b) Harness pilots** — `docs/benchmarks/POLYGLOT_PUBLIC.md` (real full runs, Aider polyglot
Python subset, n=10, seed 20260825, harness `9679fafa`):

| billed model | pass@1 | $/task | cache-read | note |
|---|---|---|---|---|
| `custom_deepseek/deepseek-v4-flash` | **9/10** | $0.161 | **0%** | 1 "no code written" fail |
| `anthropic/claude-haiku-4-5-20251001` | **10/10** | $0.235 | **70%** | — |

PILOT, n=10 — "the gap … is one task, … a coin-flip at this sample size." Full-run
extrapolation: deepseek ~$36, haiku ~$53 vs Aider's own leaderboard GPT-5-high at **$29.08**
(`[published]`, aider.chat/docs/leaderboards, read 2025-11-20). `gpt-5.4-mini` — the shipped
harness default — was **not in this pilot**.

**(c) Defaults sweep** — `MODEL_DEFAULTS_BENCH_2026-08-25.md`, harness part A (n≤3 tasks; all
6 candidates solved every task ⇒ pass rate is a tie): cache-corrected `$/solved` = deepseek
**$0.17**, gpt-5.4-mini **$0.18**, kimi $0.28, haiku $0.44, glm-5.3 $0.56, sonnet-5 $2.00.
This projection — not a coding-suite run — is what elevated `gpt-5.4-mini` to the default.

**(d) Voice/chat benches** — see §2.1/§2.2 for the tables.

---

## 2. The evidence per role

### 2.1 Voice — latency dominates

Baseline (`VOICE_LATENCY_AND_ORB_2026-08-25.md`, n=8 real turns): agent time-to-first-spoken-token
**median 7.36 s / p90 9.99 s**, which is **73 %** of the ~10.6 s speech-end→first-audio wait —
because the session model (`MiniMax-M2.7`, `reasoning:true`) thinks before every spoken word.
That is the problem the voice pack exists to fix: put a **fast, non-reasoning** model on the
first syllable.

Voice bench (`VOICE_MODEL_BENCH_2026-08-25.md`, n=20 turns × 2 runs, real 105 KB prompt + 124
tool schemas, `[MEASURED-here]`), key rows — TTFT p90 / silent-turns-in-40 / $ per turn:

| candidate | TTFT p90 | silent /40 | thinks? | $/turn |
|---|---|---|---|---|
| `claude-haiku-4-5` | **1070 ms** | **0** | no | $0.0074 |
| `deepseek-chat` | **1885 ms** | 1 | no | **$0.0018** |
| MiniMax-M2.7-highspeed | 5720 ms | 7 | yes (40/40) | $0.0064 |
| Kimi K2.5 | 6403 ms | 6 | yes | $0.0062 |
| glm-4.7-flashx | 11339 ms | 6 | yes | $0.0007 |

Only **Haiku and deepseek-chat skip the reasoning block** — the two that can hit a
sub-2 s first syllable. Everything else "thinks" and lands 5–11 s p90.

- **Primary — `custom_deepseek/deepseek-chat`** `[MEASURED-here]`. Under the p90 bar, 1
  silent turn in 40, and **4.1× cheaper** than Haiku — inside the 2× cost gate the bench was
  held to. **Why this one:** fastest first-syllable that clears the cost gate, same family as
  chat so typed and spoken turns land together.
- **Local-first — `apple_fm/apple-on-device`** `[reasoned]`. $0, prompt never leaves the Mac —
  the strongest privacy+latency story *for the first social sentence*. But 4096-token context
  and **no tool calls**: it can open "Sure, one sec —" locally, then a tool-requiring turn must
  fall back to cloud. **Never measured for voice TTFT** — this is the biggest local gap (§4).
  Second local option: `ollama/qwen3` on the M4 for longer tool-free replies.
- **Max-quality — `anthropic/claude-haiku-4-5-20251001`** `[MEASURED-here]`. Warm p90
  **1070 ms**, **zero** silent turns in 40 — the lowest-latency, most-reliable spoken turn
  measured, when the wait matters more than the 4× bill. Already one config edit away
  (`voice_model.rs:66-69`).

### 2.2 Chat — first written word fast, without losing the answer

Chat bench (`MODEL_DEFAULTS_BENCH` part B, n=30 turns, `[MEASURED-here]`) — TTFT median/p90 /
tool-pick /wordless-in-30 / quality (0=silence) / $ per turn:

| candidate | TTFT med | TTFT p90 | tool ok | wordless | quality | $/turn |
|---|---|---|---|---|---|---|
| `deepseek-chat` | **1.83 s** | **2.06 s** | 9/10 | 4 | 1.86 | $0.0278* |
| `claude-haiku-4-5` | 2.95 s | 7.53 s | 9/10 | **0** | **2.07** | $0.0085 |
| `gpt-5.4-mini` | 3.07 s | 7.29 s | **1/10** | 4 | 0.94 | $0.0392 |
| `claude-sonnet-5` | 5.09 s | 9.11 s | 8/10 | 16 | 1.40 | $0.0568 |
| `glm-5.3` (incumbent) | 14.65 s | 54.71 s | 8/10 | 12 | 0.70 | $0.0878 |

\* deepseek's $/turn is inflated by **0 % cache** on the OpenAI-format path (harness bug, fixed
#1122); post-fix it drops toward its token price (0.28/0.42 $/Mtok). `gpt-5.4-mini` picked the
right tool on **1 of 10** turns — disqualifying for chat.

- **Primary — `custom_deepseek/deepseek-chat`** `[MEASURED-here]`. The **only** candidate under
  a 2.5 s p90 first-word bar, solid tool-picking, chat-tier price. **Why:** a person watching a
  cursor blink is served fastest, at low cost, in the same family as voice.
- **Local-first — `apple_fm/apple-on-device`** (short turns) → **`ollama/qwen3`** (M4, longer)
  `[reasoned]`. $0 and private for everyday Q&A; falls back to cloud when a turn needs tools or
  long context.
- **Max-quality — `anthropic/claude-sonnet-5`** `[MEASURED-here` cost, `published` quality`]`.
  Highest-capability answer when correctness beats latency (it is slower: p90 9.1 s). Real
  ledger cost ~$0.075/call. **Why:** the chat you reach for when the answer has to be right and
  you'll wait for it.

### 2.3 Coding / harness — capability per dollar; latency irrelevant

This is where the routing principle says higher spend is justified — but *cost-conscious*, so
the pick is the cheapest model that does not lose tasks, not the smartest model available.

Evidence, ordered by strength:
- `[MEASURED-here]` **PILOT** (polyglot, real full runs, §1.5b): deepseek 9/10 @ $0.161,
  haiku 10/10 @ $0.235. **gpt-5.4-mini not run.**
- `[MEASURED-here]` projection (defaults sweep, §1.5c): cache-corrected $/solved — deepseek
  $0.17, **gpt-5.4-mini $0.18**, haiku $0.44 — but all candidates tied on pass rate at n≤3.
- `[MEASURED-here]` live ledger (§1.5a): `ollama/qwen3-coder:30b` runs at **$0** on the M4.

- **Primary — `openai/gpt-5.4-mini`** `[MEASURED-here]` PILOT + projection. Near-cheapest after
  the cache fix, a **different family** from chat/voice (so a bad day for one vendor doesn't take
  all three roles down), and the id the harness already pins. ~$0.18/task projected.
  **Honest limit:** never run head-to-head on a coding suite — this is the row §4 turns into
  "tested". A defensible measured alternative is **`custom_deepseek/deepseek-chat`** (9/10 @
  $0.161 real), which is cheaper and actually measured, at the cost of sharing the chat/voice
  family.
- **Local-first — `ollama/qwen3-coder:30b`** (M4, 16 GB) `[MEASURED-here` that it runs at $0;
  `reasoned` on quality`]`. On-device, free, private; 15 real calls already in the ledger.
  16 GB is tight (MoE, ~3 B active) and **pass rate on real tasks is unvalidated** — trust it
  for offline / privacy-sensitive coding only after the §4 eval. The M1 (headless) is the second
  host.
- **Max-quality — `anthropic/claude-opus-4.8`** (hardest tasks) / **`claude-sonnet-5`**
  (value-max) `[published`/`reasoned]`. When a wrong answer costs a re-run and the task is worth
  paying for. Matches the tiered `ModelPacks` "hard"/"edit" defaults (§3.1). Re-pin to the live
  frontier (`gpt-5.6-*`, "Opus 5") at validation time.

---

## 3. How the pack feeds the wizard / API

### 3.1 Two pack systems exist — don't conflate them

1. **Tiered `ModelPacks`** (`cost_router/packs.rs:107-148`) — struct
   `ModelPack{provider,model}`, tiers `{edit, hard, mechanical, local}`, defaults
   edit=`anthropic/claude-sonnet-5`, hard=`anthropic/claude-opus-4-8`,
   mechanical=`anthropic/claude-haiku-4-5-20251001`, local=`ollama/qwen3`. These write
   **`PERMAGENT_PACK_*`** keys and route *delegated / subagent* work — **not** the three
   user-facing roles. (Useful as the "max/local variant" source for harness, but a different
   axis.)
2. **Role-routing packs** (the user-facing "pack") — **`GET /api/packs`** → `get_packs`,
   **`POST /api/packs/apply`** → `apply_packs` (`goose-server/src/routes/packs.rs:39-89`,
   registered `routes/mod.rs:52,209`). `apply_packs` → `mappings_to_persist` →
   `set_role_model(role, provider, model)` (`packs.rs:67-81`). UI banner
   **`ui/command-center/src/components/chat/RoleRoutingPrompt.tsx`** ("Cheaper per-role routing
   is available", `api.getPacks()`/`api.applyPacks()`, lines 25/39).

**This pack targets system (2).** The three roles map to `chat_*`, `harness_*`, `voice_*` as in
§0/§1.1. Wiring caveat to verify before shipping: `apply_packs` persists via cost_router
`set_role_model`; confirm that write lands on the exact `chat_provider`/`chat_model` /
`harness_*` / `voice_*` config keys the resolvers read (`model_roles.rs`, `voice_model.rs`), or
add the mapping if it routes through the cost_router role-map instead.

### 3.2 The 2026-09-02 "GOOSE_* only on decline" ruling — NOT yet in the tree (honesty)

The task premise is that "per the 2026-09-02 ruling the wizard writes `GOOSE_*` only when the
user declines a pack." **That rule is not implemented in the current tree.** It exists only as a
*provisional plan pending Jesse's ruling* in `docs/design/AGENT_HOME_WIZARD.md` (§8 Q4, marked
**still open**; provisional plan at lines 390-395, 671, 782-783, 862-872). What the code
actually does today:

- `ui/command-center/src/components/wizard/MomentWelcome.tsx:150-155` writes `GOOSE_PROVIDER`
  (+ `GOOSE_MODEL`) as a **blanket** first-run choice — flagged in the design doc as "the single
  worst write" because it sits **above** the measured per-role defaults in precedence, so one
  first-minute pick silently becomes every role's model.
- The `RoleRoutingPrompt` banner's decline ("Not now") merely dismisses; it writes nothing.
- `GET /api/packs` self-gates to fire only when nothing per-role is configured
  (`packs.rs:52`, `should_prompt_role_routing`), and only recommends among **already-configured**
  providers (`routes/packs.rs:43`).

**Recommendation for the wizard:** implement the provisional plan — mount `RoleRoutingPrompt`
(or an equivalent) in the wizard, apply this pack's role keys on accept, and write
`GOOSE_PROVIDER`/`GOOSE_MODEL` **only when the user declines the pack** — so the measured
per-role defaults win for the common case, and the blanket session-model write survives only as
the explicit opt-out. This closes the "worst write" without removing the escape hatch. Land it
only once Jesse rules §8 Q4.

---

## 4. The honest gap: from "recommended" to "tested", and the eval that closes it

| Role | Primary is tested? | The exact gap |
|---|---|---|
| **Voice** | **Yes**, cloud primary | `deepseek-chat` measured (n=20×2). **Apple-FM local voice never measured** for TTFT. |
| **Chat** | **Yes**, cloud primary | `deepseek-chat` measured (n=30). Local (apple_fm/ollama) unmeasured; quality-max (sonnet-5) quality is `[published]`. |
| **Coding** | **No** — projection only | `gpt-5.4-mini` **never run on a coding suite**; its rank is a cache-corrected projection from a 2/3 tie. Local `qwen3-coder:30b` runs at $0 but pass rate unvalidated. |

A lightweight, runnable validation — small enough for a follow-up, sufficient to say "best,
**tested**" truthfully:

1. **Coding (the one that matters most).** Use the existing harness:
   `scripts/bench/polyglot_bench.py prepare/run/grade/report` (design:
   `HARNESS_PUBLIC_BENCHMARK_2026-08-25.md`), **n ≥ 30** Python exercises, seeded, `--max-turns 30`,
   **after** the #1122 cache fix, comparing four candidates on **pass@1 + $/task from
   `cost_ledger`**: `openai/gpt-5.4-mini` · `custom_deepseek/deepseek-chat` ·
   `anthropic/claude-sonnet-5` · `ollama/qwen3-coder:30b`. Cost/tokens/served-model come from the
   ledger, not self-report. This turns the harness Primary from PILOT-projection into a measured
   winner and grades the local variant honestly. *(SWE-bench Verified stays deferred — hardware,
   not budget; see the design doc §2.)*
2. **Voice.** Re-run the `voice_model_bench` harness (real prompt + 124 tool schemas, warm,
   **n ≥ 40**) adding the two never-measured locals — **`apple_fm/apple-on-device`** and
   **`ollama/qwen3`** — against `deepseek-chat` and `claude-haiku-4-5`, measuring
   speech-end→first-audio TTFT and silent-turn count. Closes the local-voice gap and tells us
   whether Apple FM can honestly own the first social sentence.
3. **Chat.** Spot-check (n ≥ 30 instruction/general prompts) blind-rated across
   `deepseek-chat` · `claude-sonnet-5` · a local (`apple_fm`/`ollama`), reporting p90 first-token
   + a blind quality score. Confirms the primary and grades the local fallback.

Until (1) runs, the honest public claim is: **voice and chat are "best, tested"; coding is
"best by measured projection, validation pending."**

---

## 5. Current-default vs recommended — the deltas

- **Voice / Chat / Harness Primary:** **no change.** Recommended == shipped
  (`voice_model.rs`, `model_roles.rs`). The measured, cost-gated defaults are already the
  best-fit picks; the recommendation ratifies them and states their evidence tier honestly.
- **New: named Local-first and Max-quality variants per role.** The code today expresses one
  default per role, not a three-variant pack. This doc defines the local and max columns
  (apple_fm/ollama locals; haiku/sonnet/opus maxes) so the wizard can offer "cheapest-private"
  and "best-quality" alongside the default.
- **New: honest tier labels.** The harness default carries a **projection**, not a suite
  result — surfaced here rather than presented as "tested".
- **Wizard wiring:** the "write `GOOSE_*` only on pack-decline" behavior is **not implemented**;
  it is a provisional plan (§3.2). Recommended delta: implement it and mount the role-routing
  prompt in the wizard.
- **Frontier drift:** the catalog stops at `opus-4.8`/`gpt-5.4-*`, but real usage already bills
  `gpt-5.6-terra`; re-pin the max-quality variant to the live frontier at validation time.

---

### Anchors

Repo (file:line): `model_roles.rs:23-67,130-155`; `voice_model.rs:15-77`;
`cost_router/packs.rs:52,67-81,107-148`; `goose-server/src/routes/packs.rs:39-89`;
`routes/mod.rs:52,209`; `chat/RoleRoutingPrompt.tsx:25,39`;
`wizard/MomentWelcome.tsx:150-155`; `apple_fm/mod.rs:1,92,199-204`, `sidecar.rs:352`;
`ollama.rs:28-36`; `session/spectral_schema.rs:2683-2707`; `session_manager.rs:1720-1754`;
live ledger `~/.permagent/spectral/permagent.db`.
Research: `docs/research/{VOICE_MODEL_BENCH,MODEL_DEFAULTS_BENCH,VOICE_LATENCY_AND_ORB,HARNESS_PUBLIC_BENCHMARK}_2026-08-25.md`;
`docs/benchmarks/POLYGLOT_PUBLIC.md`; `docs/design/AGENT_HOME_WIZARD.md` §2/§8.
Web (`[published]`, read 2026-09-02): artificialanalysis.ai/models; aider.chat/docs/leaderboards
(read 2025-11-20). Web verification was budget-limited this session — `[published]` claims lean
on the in-repo dated catalog and the cited harness-benchmark URLs.
</content>
</invoke>
