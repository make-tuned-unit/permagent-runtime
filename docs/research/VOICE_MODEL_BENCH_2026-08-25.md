# Voice model bench — what should answer a spoken turn (2026-08-25)

`VOICE_LATENCY_AND_ORB_2026-08-25.md` found that 73 % of the 10.6 s a voice turn
takes is the session model thinking before it says a word, and left one decision
open: **route the voice turn to a different model.** This is that decision, made
by measurement.

Harness: `crates/goose-server/src/bin/voice_model_bench.rs`, driving each
candidate through the SAME provider stack the daemon uses (`providers::create` →
`Provider::stream_split`), so `cache_control` placement, tool formatting and the
streaming decode are the daemon's. Time-to-first-sentence calls
`routes::voice::find_speakable_boundary` — the very function first audio fires on.

Inputs, none of them personal: **20 synthetic turns** (12 conversational, 5
needing one tool, 3 small planning questions —
`crates/goose-server/tests/fixtures/voice_bench_turns.json`); **the real system
prompt** from the repo's own all-extensions prompt-manager snapshot (105,480
chars) plus a synthetic 3,008-char "Live Status" tail; **the real 124 tool
schemas** (157 KB), lifted from a daemon request log by
`scripts/bench/extract_voice_tools.py`, which copies `input.tools` and nothing
else. ≈65 k input tokens per turn, the same order as the live path's ~70 k. Two
runs per candidate: run 0 cold, run 1 with the prefix cache warm.
**Total spend: $1.16.**

## Results

Milliseconds. "first sentence" is time to the first speakable boundary — what TTS
keys the first audio on. "thinks" counts turns that emitted a reasoning block.

| candidate | run | TTFT med | TTFT p90 | first sentence | total med | tool ok | cache hit | $/turn | thinks | quality |
|---|---|---|---|---|---|---|---|---|---|---|
| **MiniMax-M2.7** (today) | cold | 2580 | 6836 | 3064 | 2822 | 2/5 | 95 % | $0.0055 | 20/20 | **3.10** |
| | warm | 2839 | 4363 | 3202 | 2899 | 2/5 | 100 % | $0.0046 | 20/20 | |
| **MiniMax-M2.7-highspeed** | cold | 2450 | 5720 | 2450 | 2643 | 3/5 | 95 % | $0.0064 | 20/20 | **2.80** |
| | warm | 2516 | 4412 | 2516 | 2628 | 2/5 | 100 % | $0.0055 | 20/20 | |
| **deepseek-chat** | cold | 1452 | 1674 | 1652 | 1927 | 3/5 | n/a | $0.0181 \* | 0/20 | **2.95** |
| | warm | **1339** | **1603** | **1581** | 1911 | 4/5 | n/a | $0.0181 \* | 0/20 | |

\* An upper bound, not the real price — see "Cost" below.

Three of the four intended candidates could not be measured, and are reported as
skipped rather than quietly dropped:

- **Z.AI `glm-4.7-flashx` / `glm-4.7`** — neither `ZAI_API_KEY` nor
  `ZHIPU_API_KEY` was present in the daemon's secret store when the bench probed
  it. A key may exist elsewhere on the machine; re-probe when the keychain is
  answerable.
- **Claude Haiku 4.5** — Anthropic returns "credit balance is too low".
- **Kimi K2.5** (already a substitute for GLM) — Moonshot account suspended.

`deepseek-chat` went in instead because it is the one reachable model in the
slate that **does not reason at all** — the cleanest test of the actual thesis.

## What the numbers say

**The tail is the story, not the median.** Both MiniMax variants answer in about
2.5–2.8 s at the median but 4.4–6.8 s at p90; deepseek-chat's p90 is 1.60 s, only
0.26 s above its own median. A voice assistant is judged on its worst turns, and
a reasoning model's spread is the wait that feels unbounded.

**Highspeed is the same class of thing.** `MiniMax-M2.7-highspeed` buys ~10 % of
TTFT and costs ~20 % more per turn. It still emitted a thinking block on all 40
turns. It is not the lever.

**Silence is worse than slowness.** MiniMax opened with a bare tool call — no
spoken text at all — on 6/20 (baseline) and 7/20 (highspeed) turns, including
plain conversational ones. deepseek-chat did that once in 20, and otherwise spoke
first ("Let me check your current projects.") *while* calling the tool. In
production a silent opening means the user hears nothing until the tool returns
AND a second model round-trip starts, which no median in this table includes.

**Tool choice is mediocre across the board** — best 4/5, with a generic
`observe_app` recurring where `project_list` or the inbox tool was wanted. A
prompt/tool-surface problem, not a model choice, and unchanged by this decision.

**Quality is a wash.** Blind, shuffled, per-turn rubric scoring put all three
within 0.3 on a 0–5 scale (baseline 3.10, deepseek 2.95, highspeed 2.80).
deepseek-chat never scored a 0; MiniMax scored 0 on 2 and 3 turns, all of them
silent tool calls on turns that only wanted an answer. deepseek's own tic is
meta-commentary ("That's a constraint problem, not a phone problem"), which cost
it several 5s.

**Cost.** The $0.0181/turn for deepseek-chat is an artefact this bench uncovered
and this PR fixes: `formats::openai::get_usage` read only the Anthropic-style
`cache_read_input_tokens`, so DeepSeek's automatic context cache
(`prompt_cache_hit_tokens`) and OpenAI's own `prompt_tokens_details.cached_tokens`
were both invisible — every cached turn on every OpenAI-format provider billed as
a cold prefill. At the canonical $0.028/M cache-read rate over a ~64.8 k cached
prefix the arithmetic gives ≈$0.002/turn, i.e. **cheaper than MiniMax**. A
projection, not a measurement — see "Not measured".

## Recommendation

**Route voice turns to `custom_deepseek` / `deepseek-chat`.** Against the stated
budget:

- *First audio ≤ 2.5 s* — approached, not reached. This bench's 1581 ms
  first-sentence plus the rest of the measured pipeline (500 ms endpointing after
  `feat/voice-endpointing-and-orb`, 116 ms STT, 142 ms pre-stream, 896 ms Kokoro)
  is **≈3.2 s** speech-end→first-audio, against ≈4.9 s for MiniMax warm and
  ≈10.6 s today. The last 700 ms is a first-chunk-length problem, not a model one.
- *Tool calls correct* — 3–4/5, best of the three, and the only candidate that
  speaks while it calls.
- *Quality not markedly below baseline* — 2.95 vs 3.10, inside the noise of a
  20-turn sample, with fewer catastrophic turns.

## Shipped (this PR)

`voice_provider` / `voice_model` in `~/.permagent/config.yaml`, resolved by
`crates/goose/src/config/voice_model.rs` and applied by
`routes::voice::apply_voice_model` — on the voice reply path and nowhere else.
The chat path keeps `GOOSE_PROVIDER`/`GOOSE_MODEL`, untouched.

```yaml
voice_provider: custom_deepseek   # both keys, or neither
voice_model: deepseek-chat
```

- **Unset** → the measured default applies. A deliberate departure from the role
  map's no-baked-default rule: the voice path has a measured winner and a user
  who is waiting out loud.
- **Half set** → the default applies and the daemon WARNs; half-configured is a
  typo, not an intention.
- **`voice_model: session`** (or `off` / `none`) → voice runs on the session
  model: exactly the pre-bench behaviour.
- **Unreachable** (bad id, missing key, no network) → the daemon logs a warning
  and the turn runs on the session model. A voice model that cannot be reached
  must never become a failed turn.

Editable in Settings → Models, under the primary model readout. Two other bench
findings ship with it: **the silence rule** is now in `VOICE_REPLY_STYLE` (voice
path only) — never open a turn in silence, say one short sentence before or
alongside the tool call, which is model-independent and the largest
perceived-latency win in the data; and **the OpenAI-format prompt-cache fix**
described under Cost.

## Reproducing

```bash
python3 scripts/bench/extract_voice_tools.py ~/.permagent/logs/llm_request.<n>.jsonl /tmp/voice_bench_tools.json
BENCH="cargo run --release -p permagent-daemon --bin voice_model_bench --"
$BENCH --probe     # which providers are configured (presence only, never a value)
$BENCH --dry-run   # projected spend, no requests
$BENCH --run --runs 2 --candidates "baseline=minimax/MiniMax-M2.7,dschat=custom_deepseek/deepseek-chat" \
       --judge openai/gpt-5.4-mini --out results.json
$BENCH --judge-only results.json --judge openai/gpt-5.4-mini   # re-score without re-spending
```

The run refuses to start if projected spend exceeds `--budget-usd` (default $5),
refuses outright to spend on a model with no canonical price, and stops mid-run
the moment MEASURED spend reaches the budget.

## Not measured

- **Z.AI GLM, Claude Haiku 4.5, Kimi** — no key / no credit (table above). Haiku
  is the one worth revisiting: it is the only candidate that would not think by
  default *and* has full prompt-cache support on the path we already use.
- **The honest DeepSeek price, and the automated judge.** Both are blocked the
  same way: every API-key read now times out at 30 s against a pending macOS
  keychain authorisation prompt only an interactive session can answer. The
  harness supports `--judge` / `--judge-only` (a re-score costs 20 small calls);
  the quality column here came instead from blind, shuffled, per-turn rubric
  scoring by an Anthropic-family agent — cross-family to both MiniMax and
  DeepSeek, but not reproducible by the harness.
- **Multi-turn conversations.** Every bench turn is a single user message; the
  live 7.4 s median came from turns carrying 8–16 prior ones. Every absolute
  number here is therefore optimistic — the *relative* gap is the finding.
- **The second round-trip after a tool call.** The bench never executes a tool,
  so no candidate pays what a tool call actually costs.
- **Real audio.** Nothing went through a microphone; STT, endpointing and TTS
  belong to `feat/voice-endpointing-and-orb`.
- **Whether a smaller tool surface helps.** 124 schemas ride on every spoken "how
  are you". They are cached, so the cost is small — but they are also what every
  candidate was choosing wrongly among.
