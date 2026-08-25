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
**Total spend: $1.68** across two sessions (the second added Z.AI and Haiku once
a stuck keychain prompt was cleared).

## Results

Milliseconds. "first sentence" is time to the first speakable boundary — what TTS
keys the first audio on. "thinks" counts turns that emitted a reasoning block.
"quality" is one blind, shuffled, per-turn score from `openai/gpt-5.4-mini` — a
family none of the candidates belongs to.

| candidate | run | TTFT med | TTFT p90 | first sentence | total med | tool ok | silent | cache hit | $/turn | thinks | quality |
|---|---|---|---|---|---|---|---|---|---|---|---|
| MiniMax-M2.7 (before) | cold | 2619 | 6836 | 3124 | 2822 | 2/5 | 6/20 | 95 % | $0.0055 | 20/20 | **2.35** |
| | warm | 3202 | 4363 | 3267 | 2899 | 2/5 | 6/20 | 100 % | $0.0046 | 20/20 | |
| MiniMax-M2.7-highspeed | cold | 2450 | 5720 | 2450 | 2643 | 3/5 | 7/20 | 95 % | $0.0064 | 20/20 | **2.50** |
| | warm | 2516 | 4412 | 2516 | 2628 | 2/5 | 7/20 | 100 % | $0.0055 | 20/20 | |
| **deepseek-chat** | cold | 1515 | 1786 | 1833 | 2100 | 3/5 | 3/20 | 100 % | $0.0018 | 0/20 | **2.60** |
| | warm | **1580** | **1885** | **1798** | 2071 | **4/5** | **1/20** | 100 % | **$0.0018** | 0/20 | |
| Z.AI glm-4.7-flashx | cold | 3627 | 7949 | 3694 | 4367 | 4/5 | 5/20 | 100 % | $0.0009 | 20/20 | **2.15** |
| | warm | 4050 | 11339 | 4319 | 4865 | 4/5 | 6/20 | 100 % | $0.0007 | 20/20 | |
| Z.AI glm-4.7 | cold | 7866 | 28678 | 8217 | 8703 | 2/5 | 4/20 | 100 % | $0.0105 | 20/20 | **2.35** |
| | warm | 6046 | 10752 | 6297 | 6364 | 4/5 | 5/20 | 100 % | $0.0074 | 20/20 | |

"silent" counts turns that produced a tool call and no spoken text at all — the
user hears nothing for a whole tool round trip and cannot tell it from a crash.

MiniMax was measured in the first session, the rest in the second; the two are
comparable because MiniMax runs on the Anthropic format, whose usage parsing
already read cache fields correctly. A repeat of deepseek-chat in the first
session gave 1339 ms / 1603 ms, so read its numbers as a 1.34–1.58 s median band.

**Claude Haiku 4.5 still could not be measured:** the key is configured, but
Anthropic answers every request with "credit balance is too low". It remains the
candidate most worth revisiting — it is the only one that would not think by
default *and* has full prompt-cache support on the path we already use. (Kimi
K2.5 was tried in the first session and the Moonshot account is suspended.)

## What the numbers say

**One model does not think, and it is the only one that is fast.** Every other
candidate emitted a reasoning block on all 40 of its turns, and every other
candidate is 1.5–4× slower to the first spoken word. This is the whole finding:
the lever is not a faster endpoint for a reasoning model, it is a model that does
not stop to reason before saying hello.

**The tail is the story, not the median.** deepseek-chat's p90 sits 300 ms above
its own median. Everything else spreads: MiniMax 4.4–6.8 s, glm-4.7-flashx up to
11.3 s, glm-4.7 up to 28.7 s. Both Z.AI models also opened their first cold turn
at 64–70 s — a queue or a cold start, but on a voice path that is a hang. A voice
assistant is judged on its worst turns.

**`MiniMax-M2.7-highspeed` is the same class of thing.** ~10 % off TTFT for ~20 %
more per turn, still thinking on every turn. It is not the lever.

**The cheapest model is not the answer either.** glm-4.7-flashx costs a quarter
of deepseek-chat and picks tools just as well (4/5), but it thinks, its median is
2.5× worse and its p90 is 6×. At these prices the difference is fractions of a
cent per spoken turn; latency is the scarce resource here, not money.

**Silence is worse than slowness.** Every candidate opened some turns with a bare
tool call and no spoken text — 4 to 7 turns in 20 — including plain conversational
ones. deepseek-chat was the best at 1–3, and it typically spoke first ("Let me
check your current projects.") *while* calling the tool. A silent opening means
the user hears nothing until the tool returns AND a second model round-trip
starts, which no median in this table includes. That is model-independent enough
to fix in the prompt, and this PR does.

**Tool choice is mediocre across the board** — 2/5 to 4/5, with a generic
`observe_app` recurring where `project_list` or the inbox tool was wanted. A
prompt/tool-surface problem, not a model choice, and unchanged by this decision.

**Quality is a wash, with the fastest model nominally ahead.** The cross-family
judge (`openai/gpt-5.4-mini`, blind and shuffled per turn) put all five inside
0.45 of each other on 0–5: deepseek-chat 2.60, highspeed 2.50, MiniMax-M2.7 and
glm-4.7 2.35, glm-4.7-flashx 2.15. An earlier hand-scored pass with a different
judge ranked MiniMax-M2.7 first by 0.15 — so treat the ordering as noise and the
*absence of a quality cliff* as the finding. The scores are low across the board
because the rubric punishes silent turns and written-prose formatting, which
every candidate did.

**Cost.** deepseek-chat's first-session figure of $0.0181/turn was an artefact
this bench uncovered and this PR fixes: `formats::openai::get_usage` read only the
Anthropic-style `cache_read_input_tokens`, so DeepSeek's automatic context cache
(`prompt_cache_hit_tokens`) and OpenAI's own `prompt_tokens_details.cached_tokens`
were both invisible and every cached turn on every OpenAI-format provider billed
as a cold prefill. With the fix in, the same run measures **$0.0018/turn at a
100 % cache-hit rate** — a tenth of the pre-fix number, and cheaper than the
MiniMax model it replaces. Both Z.AI models likewise only report a cache hit at
all because of this fix.

## Recommendation

**Route voice turns to `custom_deepseek` / `deepseek-chat`.** It wins every column
that matters here — fastest median, by far the tightest tail, fewest silent turns,
joint-best tool correctness, top quality score — and it is cheaper than the model
it replaces. Against the stated budget:

- *First audio ≤ 2.5 s* — approached, not reached. This bench's 1798 ms
  first-sentence plus the rest of the measured pipeline (500 ms endpointing after
  `feat/voice-endpointing-and-orb`, 116 ms STT, 142 ms pre-stream, 896 ms Kokoro)
  is **≈3.5 s** speech-end→first-audio, against ≈4.9 s for MiniMax warm and
  ≈10.6 s today. The remainder is a first-chunk-length problem, not a model one.
- *Tool calls correct* — 4/5 warm, joint-best, and the only candidate that
  reliably speaks while it calls.
- *Quality not markedly below baseline* — 2.60 vs 2.35, nominally ahead, and
  inside the noise of a 20-turn sample either way.

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

- **Claude Haiku 4.5 and Kimi K2.5** — key present, no credit; Moonshot account
  suspended. Haiku is the one worth revisiting: the only candidate that would not
  think by default *and* has full prompt-cache support on the path we already use.
- **What the Z.AI cold-start spikes actually were.** 64–70 s on the first turn of
  each Z.AI run, and 28.7 s once mid-run. Queue, cold start or rate limit — the
  bench cannot tell, and on a voice path the distinction does not matter.
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
