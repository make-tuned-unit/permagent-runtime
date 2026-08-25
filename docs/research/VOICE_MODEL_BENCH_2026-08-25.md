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
**Total spend: $2.45** across three sessions — Z.AI joined once a stuck keychain
prompt was cleared, Haiku and Kimi once their accounts were topped up.

## Results

Milliseconds. "first sentence" is time to the first speakable boundary — what TTS
keys the first audio on. "silent" counts turns that produced a tool call and no
spoken text at all. "thinks" counts turns that emitted a reasoning block.

| candidate | run | TTFT med | TTFT p90 | first sentence | total med | tool ok | silent | cache hit | $/turn | thinks | quality |
|---|---|---|---|---|---|---|---|---|---|---|---|
| **Claude Haiku 4.5** (default) | cold | 931 | 1090 | 1191 | 1475 | 4/5 | **0/20** | 94 % | $0.0125 | 0/20 | 2.55 |
| | warm | **856** | **1070** | **1136** | 1497 | 4/5 | **0/20** | 100 % | $0.0074 | 0/20 | |
| **deepseek-chat** | cold | 1515 | 1786 | 1833 | 2100 | 3/5 | 3/20 | 100 % | $0.0018 | 0/20 | 2.60 |
| | warm | 1580 | 1885 | 1798 | 2071 | 4/5 | 1/20 | 100 % | **$0.0018** | 0/20 | |
| MiniMax-M2.7 (before) | cold | 2619 | 6836 | 3124 | 2822 | 2/5 | 6/20 | 95 % | $0.0055 | 20/20 | 2.70 |
| | warm | 3202 | 4363 | 3267 | 2899 | 2/5 | 6/20 | 100 % | $0.0046 | 20/20 | |
| MiniMax-M2.7-highspeed | cold | 2450 | 5720 | 2450 | 2643 | 3/5 | 7/20 | 95 % | $0.0064 | 20/20 | 2.40 |
| | warm | 2516 | 4412 | 2516 | 2628 | 2/5 | 7/20 | 100 % | $0.0055 | 20/20 | |
| Kimi K2.5 | cold | 4536 | 6010 | 4717 | 5059 | 4/5 | 6/20 | 100 % | $0.0078 | 20/20 | 2.95 |
| | warm | 3627 | 6403 | 3734 | 4201 | 4/5 | 6/20 | 100 % | $0.0062 | 20/20 | |
| Z.AI glm-4.7-flashx | cold | 3627 | 7949 | 3694 | 4367 | 4/5 | 5/20 | 100 % | $0.0009 | 20/20 | 2.70 |
| | warm | 4050 | 11339 | 4319 | 4865 | 4/5 | 6/20 | 100 % | **$0.0007** | 20/20 | |
| Z.AI glm-4.7 | cold | 7866 | 28678 | 8217 | 8703 | 2/5 | 4/20 | 100 % | $0.0105 | 20/20 | 2.75 |
| | warm | 6046 | 10752 | 6297 | 6364 | 4/5 | 5/20 | 100 % | $0.0074 | 20/20 | |

Quality is one blind, shuffled, per-turn score from `openai/gpt-5.4-mini` — a
family none of the candidates belongs to. Read it as comparative within this
slate, not absolute: the same answers re-judged against a smaller slate moved by
up to 0.55, which is the whole spread. The finding is the *absence of a quality
cliff*, not the ordering.

MiniMax and the Z.AI models were measured in earlier sessions than Haiku and
Kimi. The runs are comparable — the prompt, the tool set and the harness are
byte-identical, and the one code change between them (the OpenAI-format
cache-field fix) only ever made a cached turn *cheaper to report*, never faster.
A repeat of deepseek-chat across two sessions gave 1339–1580 ms median, so read
its latency as a band rather than a point.

## What the numbers say

**Only two of the seven do not think, and they are the only two that are fast.**
Claude Haiku 4.5 and deepseek-chat emitted no reasoning block on any of their 40
turns. Every other candidate emitted one on all 40, and every other candidate is
2–4× slower to the first spoken word. That is the whole finding: the lever is not
a faster endpoint for a reasoning model, it is a model that does not stop to
reason before saying hello.

**The tail is the story, not the median.** Haiku's p90 sits 214 ms above its own
median and deepseek-chat's 305 ms above its. Everything else spreads: MiniMax
4.4–6.8 s, Kimi 6.0–6.4 s, glm-4.7-flashx up to 11.3 s, glm-4.7 up to 28.7 s.
Both Z.AI models also opened their first cold turn at 64–70 s. A voice assistant
is judged on its worst turns, and on a voice path a 28 s turn is a hang.

**Haiku is the latency ceiling, and it never goes silent.** 856 ms to first
token, 1136 ms to a speakable sentence, 4/5 tools, 100 % cache hit on the warm
run — and **zero** turns in 40 that opened with a bare tool call and no words,
where every other candidate managed 1 to 7. It costs 4.1× deepseek-chat per turn,
which is the whole of the case against it.

**`MiniMax-M2.7-highspeed` is the same class of thing as the model it varies.**
~10 % off TTFT for ~20 % more per turn, still thinking on every turn.

**The cheapest model is not the answer.** glm-4.7-flashx costs a quarter of
deepseek-chat and picks tools just as well, but it thinks, its median is 2.5×
worse and its p90 6×. At these prices the spread across the whole slate is
fractions of a cent per spoken turn; latency is the scarce resource, not money.

**Silence is worse than slowness.** Five of seven candidates opened 4–7 turns in
20 with a bare tool call and no spoken text, including plain conversational ones.
The user then hears nothing until the tool returns AND a second model round-trip
starts — which no median in this table includes — and cannot tell it from a
crash. Only Haiku (0) and deepseek-chat (1 warm) largely avoided it, typically by
speaking first ("Let me check your current projects.") *while* calling the tool.
That is model-independent enough to fix in the prompt, and this PR does.

**Tool choice is mediocre across the board** — 2/5 to 4/5, with a generic
`observe_app` recurring where `project_list` or the inbox tool was wanted. A
prompt/tool-surface problem, not a model choice, and unchanged by this decision.

**Cost.** deepseek-chat's first-session figure of $0.0181/turn was an artefact
this bench uncovered and this PR fixes: `formats::openai::get_usage` read only the
Anthropic-style `cache_read_input_tokens`, so DeepSeek's automatic context cache
(`prompt_cache_hit_tokens`) and OpenAI's own `prompt_tokens_details.cached_tokens`
were both invisible and every cached turn on every OpenAI-format provider billed
as a cold prefill. With the fix in, the same run measures **$0.0018/turn at a
100 % cache-hit rate** — a tenth of the pre-fix number. Both Z.AI models likewise
only report a cache hit at all because of this fix. Haiku's 94 %→100 % confirms
the Anthropic path's `cache_control` placement was already correct for it.

## Recommendation

**Route voice turns to `anthropic` / `claude-haiku-4-5-20251001`.** It wins the
two things that matter most on a voice path: a p90 of 1070 ms against
deepseek-chat's 1885 ms, and **zero** silent turns in 40 against one. It also
takes 4/5 tool turns, holds a 100 % cache-hit rate on the warm run, and never
emits a reasoning block.

It is not the cheapest. At $0.0074/turn it is ~4× deepseek-chat's $0.0018 — at a
hundred spoken turns a day, $0.74 against $0.18. Jesse took that trade for
~700 ms off every spoken reply and the dead air gone. Anyone who wants the bill
smaller than the wait short has the alternative documented and one edit away:

```yaml
voice_provider: custom_deepseek
voice_model: deepseek-chat
```

Against the stated budget, the shipped default:

- *First audio ≤ 2.5 s* — nearly. This bench's 1136 ms first-sentence plus the
  rest of the measured pipeline (500 ms endpointing after
  `feat/voice-endpointing-and-orb`, 116 ms STT, 142 ms pre-stream, 896 ms Kokoro)
  is **≈2.8 s** speech-end→first-audio, against ≈10.6 s today. No other
  configuration in this bench comes close (deepseek-chat: ≈3.5 s; MiniMax warm:
  ≈4.9 s).
- *Tool calls correct* — 4/5, joint-best across the slate.
- *Quality not markedly below baseline* — 2.55 against MiniMax-M2.7's 2.70, well
  inside the ±0.55 the judge moved when the slate changed. Note this is the one
  column where Haiku does not lead; it is also the column this bench trusts least
  (see above).

## Shipped (this PR)

`voice_provider` / `voice_model` in `~/.permagent/config.yaml`, resolved by
`crates/goose/src/config/voice_model.rs` and applied by
`routes::voice::apply_voice_model` — on the voice reply path and nowhere else.
The chat path keeps `GOOSE_PROVIDER`/`GOOSE_MODEL`, untouched.

```yaml
voice_provider: anthropic                  # both keys, or neither
voice_model: claude-haiku-4-5-20251001
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

- **What the Z.AI cold-start spikes actually were.** 64–70 s on the first turn of
  each Z.AI run, and 28.7 s once mid-run. Queue, cold start or rate limit — the
  bench cannot tell, and on a voice path the distinction does not matter.
- **Whether Haiku holds its lead under load.** Both its runs were quiet and
  sequential. Nothing here says what its p90 does when the account is busy, and a
  4× price gap deserves that answer before it becomes the default.
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
- **Anything on-device.** Every candidate is a hosted API. A local model on the
  M4 would remove the network leg entirely and is the obvious next question.
