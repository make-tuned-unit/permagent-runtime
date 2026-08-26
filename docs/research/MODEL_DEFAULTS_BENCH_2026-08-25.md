# Model defaults — which model codes, and which model chats (2026-08-25)

`VOICE_MODEL_BENCH_2026-08-25.md` settled one of three model questions by
measurement. This settles the other two:

- **The coding harness** — best task performance at the lowest cost.
- **Chat** — lowest latency at the best quality.

Same method as the voice bench, deliberately: the real prompt, the real tool
surface, the real provider stack, canonical prices, a fail-closed budget guard,
and a cross-family judge. Where this bench had to depart from that method it
says so in place rather than in a footnote.

## What was measured, and with what

**Candidates.** Nine were probed live and all nine answered: Claude Haiku 4.5,
Claude Sonnet 5, Z.AI GLM-5.3 and GLM-4.7, MiniMax-M2.7, deepseek-chat,
deepseek-reasoner, Kimi K2.5, gpt-5.4-mini. Kimi's Moonshot account and the
Anthropic balance — both dead during the voice bench — are live again.

Six went into the harness sweep. GLM-4.7 was cut as superseded by GLM-5.3 (the
incumbent, which had to stay), MiniMax-M2.7 as already rejected on the voice
bench, and deepseek-reasoner as the same vendor and the same published rate as
deepseek-chat. That cut is a budget decision, not a measurement, and it is
recorded here so nobody reads their absence as a result.

**Prices** come from the repo's canonical table
(`crates/goose/src/providers/canonical/data/canonical_models.json`) and its
hand-maintained fallback (`published_prices.rs`), never from memory. A model with
no canonical price is refused rather than costed at zero.

## Part A — the coding harness

### Method

Ten small, real coding tasks against one throwaway 69-file polyglot repository:
four TypeScript, three Rust, three Python; 5–40 lines of change each; three easy,
five medium, two hard. Two carry a specific load:

- one names a symbol but not its file, in a tree that also contains a same-named
  dead-code decoy — it cannot be solved without the repo map or a search;
- one can only be solved by RUNNING the failing test, because the rounding rule
  it needs appears nowhere except in the assertion message.

Each task ships an `oracle/` that permagent-eval copies **over** the finished
workspace before grading, so the agent never saw the test and could not have
weakened it. Every task was verified through that exact path before any model ran:
a clean checkout FAILS and the reference solution PASSES, on all ten.

Runs go through the real harness — `permagent run --recipe permagent-coding`,
the shipped signed CLI, `GOOSE_MODE=auto`, `--max-turns 25`, an isolated
`PERMAGENT_PATH_ROOT` per run so the cost ledger is exactly that run — with the
cost-router packs pinned to the candidate so sub-agent and reviewer work stays on
the model under test. The recipe is left intact, including its independent-review
step; what is measured is the harness on a model, not the model alone.

The sweep is **task-major with a spend stop at row boundaries**: every candidate
runs task 1, then every candidate runs task 2. If the budget stops it, what
survives is a complete N-way comparison on fewer tasks rather than a ragged one
on all ten. It stopped, and the results table says where.

### Two things this bench found before it measured anything

**`permagent-eval` could never have run a single task.** `build_invocation`
emitted `permagent run --recipe <r> … -t <prompt>`, but `--recipe`, `-t/--text`
and `-i/--instructions` are declared mutually exclusive on `InputOptions`
(`crates/goose-cli/src/cli.rs:188-220`). clap rejects the argv before the process
does anything: *"the argument '--recipe' cannot be used with '--text'"*. A
headless recipe run takes its user prompt from the recipe's own `prompt:` field
(`parse_run_input`, `cli.rs:1421`), so the fix is to write a per-task recipe file
— the rendered coding recipe plus a `prompt:` block, title preserved verbatim
because `is_coding_harness_recipe` matches on the title and that is what gates
repo-map injection. Fixed in this PR, with a test asserting the argv never
carries both flags again.

**The prompt is the cost.** One run of the cheapest candidate on the easiest task
— a one-line fix, solved — spent 26 provider calls and **719,042 input tokens**
against 6,633 output tokens. That is the ~17k-token capability inventory plus 100+
tool schemas riding on all 26 calls. Nothing in this table is really a statement
about model pricing; it is a statement about a prompt that is resent every turn.

### Results

Costs are spend on **the model under test**; see "leaked" below. `$/solved` is
spend over all attempted tasks divided by tasks solved, so a failed attempt is
amortised into the price of a success. Two complete six-way rows landed before
the spend stop; a third, harder task ran for the three candidates the decision
turned on.

| candidate | billed as | solved | $ measured | $/solved | $ cache-corrected | leaked $ | med wall | med calls | med tools | input tok | output tok | cache read |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **haiku** | `claude-haiku-4-5-20251001` | 2/2 | $0.51 | **$0.25** | $0.44 | — | **41 s** | 23 | 21 | 1349k | 8k | **79%** |
| gpt54mini | `gpt-5.4-mini-2026-03-17` | 2/2 | $0.55 | $0.27 | $0.18 | $1.51 | 90 s | 16 | 18 | 682k | 8k | 0% |
| dschat | `deepseek-v4-flash` | 2/2 | $0.65 | $0.32 | $0.17 | — | 84 s | 25 | 23 | 1428k | 15k | 0% |
| kimi25 | `kimi-k2.5` | 2/2 | $0.75 | $0.37 | $0.28 | — | 123 s | 22 | 22 | 1189k | 11k | 0% |
| glm53 *(incumbent)* | `glm-5.3` | 2/2 | $1.38 | $0.69 | $0.56 | — | 489 s * | 18 | 17 | 915k | 23k | 0% |
| sonnet5 | `claude-sonnet-5` | 2/2 | $2.33 | $1.16 | $2.00 | — | 355 s | 24 | 22 | 1936k | 20k | **78%** |

\* GLM-5.3 hit the 600 s wall-clock ceiling on one task and was killed. Its change
was already correct, so the oracle graded it PASS — but its wall time is a lower
bound, not a measurement.

Per task:

| task | haiku | dschat | kimi25 | gpt54mini | sonnet5 | glm53 |
|---|---|---|---|---|---|---|
| py-percentage (easy) | PASS 39 s $0.25 | PASS 70 s $0.30 | PASS 109 s $0.33 | PASS 42 s $0.23 | PASS 564 s $1.18 | PASS 600 s $0.73 |
| ts-gold-discount (needs the map) | PASS 42 s $0.26 | PASS 99 s $0.35 | PASS 137 s $0.42 | PASS 137 s $0.31 | PASS 147 s $1.15 | PASS 378 s $0.65 |
| rs-parse-amount (hard) | **PASS 26 s $0.14** | PASS 65 s $0.24 | — | — | — | PASS 401 s $0.49 |

Two ids in that table are **aliases**, and the ledger is the only place that says
so: `deepseek-chat` bills as `deepseek-v4-flash`, and `gpt-5.4-mini` bills as
`gpt-5.4-mini-2026-03-17`. Rows are labelled by what was billed, not by what was
asked for.

### Reading it

**Pass rate decides nothing here, and pretending otherwise would be dishonest.**
Every candidate solved every task it was given, including the one with the
same-named decoy and the hard API-signature change. With n ≤ 3 that is a tie, not
a ranking, and the differences that separate these models are cost, wall time and
tool-call count — which are far less noisy at this sample size. The remaining
seven fixture tasks are in the repo and cost about $3.60 per six-way row to run.

**Haiku is cheapest and fastest, on the easy tasks and on the hard one.** $0.25
per solved task against $0.69 for the incumbent GLM-5.3 and $1.16 for Sonnet 5;
41 s median against 489 s and 355 s. On the hard task — an API signature change
threaded through two call sites — it finished in 26 s for $0.14 while GLM-5.3
took 401 s. Note that it does NOT get there by doing less: it made 23 provider
calls and 21 tool calls to GLM-5.3's 18 and 17. It does the same amount of work,
twelve times faster and for a third of the money.

**The expensive models are not buying anything here — they are being amplified.**
Sonnet 5 spent nine and a half minutes and $1.18 making a one-line change Haiku
made in 39 seconds. The harness runs a verify loop, delegates, and summons an
independent reviewer; every one of those steps costs a round trip, so the metric
that matters is cost per solved task, not the model's headline rate.

**Haiku wins today partly because it is the only one whose cache works.** It read
79% of its input from the prompt cache; Sonnet 5 78%; every OpenAI-format
candidate got **0%**, a prefix-stability bug being fixed separately. The
`$ cache-corrected` column projects each candidate at the 78% share Anthropic
actually achieved on these same tasks, at the billed model's own cache-read rate.
Corrected, deepseek ($0.17) and gpt-5.4-mini ($0.18) come out CHEAPER than Haiku
($0.44). That column is a projection, clearly labelled, and it is the reason the
recommendation below says to re-run this bench once that fix lands.

**The harness delegates to a model you did not pin.** Two runs quietly billed a
third model: gpt-5.4-mini's map task made 15 of its own calls at $0.31 plus
**three calls to `anthropic/claude-fable-5` at $1.51**, and GLM-5.3's hard task
made 13 own calls at $0.49 plus **six at $2.20** — 82% of that run's spend. Both
runs had every `PERMAGENT_PACK_{EDIT,HARD,MECHANICAL,LOCAL}_{PROVIDER,MODEL}`
pinned to the candidate, and both were runs where the model used `delegate`. So
`summon`/`delegate` does not honour the pack pins: an operator who switches to a
cheaper model to spend less can still send most of a task's money to Anthropic
without being asked. Kept out of the candidates' numbers as the `leaked $` column,
and being fixed in `fix/delegate-honours-model-pins` — not here.

**One column was measured badly and is therefore absent.** The first version of
the sweep counted rate-limit events with a pattern that also matched a bare
`429`, which appears inside token counts; it reported 27 rate-limit events for a
13-call run. The number is not in this table because it was not measured.
`permagent_eval::harness_log::scan`, added in this PR, does it properly — it
matches the CLI's actual `▸` tool banner and the real turn-limit sentence, with
the source line for each marker quoted in its doc comment.

### Recommendation — harness

**`harness_provider: anthropic` / `harness_model: claude-haiku-4-5-20251001`.**
Cheapest per solved task, fastest by 2–12x, solved everything the others solved
including the hard one, and 2.8x cheaper and 12x faster than the incumbent
GLM-5.3 for the same result.

**Provisional, and deliberately so.** Haiku's cost lead rests on being the only
candidate whose prompt cache works on this path today. Corrected for that bug,
two candidates project cheaper. **Re-run this bench once
`fix/harness-cache-prefix-and-reviewer` lands, before treating this default as
settled.**

## Part B — chat

### Method and its one honest compromise

Thirty synthetic turns — 15 conversational, 10 needing one tool, 5 short
reasoning — in `crates/goose-server/tests/fixtures/chat_bench_turns.json`.
Nothing personal: invented prompts, placeholder project names, a fabricated
"Live Status" tail so the stable/volatile prompt-cache split is exercised the way
the daemon does it. The system prompt is measured **as-is**, capability inventory
included, per #1118.

**The compromise.** The purpose-built bench for this is
`crates/goose-server/src/bin/chat_model_bench.rs` in this PR, which drives
`providers::create` → `Provider::stream_split` — the daemon's own entrypoint. It
could not be run here. A freshly built dev binary gets an ad-hoc, content-hashed
code identity, so macOS treats it as a new application and puts up a keychain
dialog the first time it reads a secret; `--probe` hangs on that dialog, and so
does `security find-generic-password` (see the `permagent-keychain-prompts`
note — the identity changes on every rebuild, so no "Always Allow" survives). An
unattended run cannot clear it.

So chat was measured through the **signed bundled CLI** instead:
`permagent run --no-session --output-format stream-json --provider … --model …`,
with the full extension profile loaded. Both paths go through `Agent::reply` →
`prepare_tools_and_prompt`, so the provider stack, the capability inventory, the
tool schemas and the cache-control layout are identical. What differs: this is the
CLI session prompt, not `/reply`'s, so it lacks the daemon's ambient-context and
brain-recall injection — and every turn pays CLI process startup, which `/reply`
does not. **That startup is measured separately per candidate and reported as its
own column; it is not subtracted from anything.** Read the latency numbers as
comparable between candidates and pessimistic in absolute terms.

Turns run as one sequence per candidate so the provider-side prefix cache warms
the way it does in a real conversation: turn 1 is the cold number, turns 2–30 the
warm ones.

### Results

Thirty turns per candidate, milliseconds. TTFT includes the CLI startup in its
own column. `$/turn` is measured on the 20 non-tool turns only — a tool turn is
refused before its usage reaches the ledger — so it understates a real chat mix.

| candidate | TTFT med | **TTFT p90** | first sentence | total med | startup | tool ok | wordless | thinks | $/turn | cache read |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| **haiku** | 2945 | **7528** | 3240 | 4120 | 1935 | 9/10 | **0/30** | 0/30 | **$0.0085** | **99%** |
| dschat | **1832** | **2060** | 2066 | 2336 | 1791 | 9/10 | 4/30 | 0/30 | $0.0278 | 0% |
| gpt54mini | 3070 | 7294 | 3070 | 3388 | 2586 | **1/10** | 4/30 | 11/30 | $0.0392 | 0% |
| kimi25 | 4419 | 6636 | 4526 | 4680 | 1771 | 9/10 | 11/30 | 30/30 | $0.0348 | 0% |
| sonnet5 | 5093 | 9110 | 5638 | 4721 | 61578 † | 8/10 | 16/30 | 0/30 | $0.0568 | 92% |
| glm53 | 14651 | **54710** | 14656 | 17049 | 2323 | 8/10 | 12/30 | 20/30 | $0.0878 | 0% |

† Sonnet 5's startup probe hit a provider retry and is not a startup measurement;
read the other five (1.8–2.6 s) as the range.

TTFT median by turn kind:

| candidate | conversational | tool | reasoning |
|---|---:|---:|---:|
| haiku | 2590 | 3738 | 2517 |
| dschat | 1781 | 1878 | 1947 |
| gpt54mini | 3463 | 2910 | 3202 |
| kimi25 | 3764 | 4155 | 5139 |
| sonnet5 | 2586 | 5695 | 5731 |
| glm53 | 12253 | 20805 | 33750 |

Quality: one blind, per-turn, position-rotated score from `openai/gpt-5.4-mini` —
a family none of the candidates belongs to **except gpt-5.4-mini itself**, whose
row is same-family and should be read with that in mind.

| candidate | quality where it answered | turns answered | wordless | **quality over all 30 (wordless = 0)** |
|---|---:|---:|---:|---:|
| **haiku** | 2.14 | 29 | 0 | **2.07** |
| dschat | 2.15 | 26 | 4 | **1.86** |
| kimi25 | 2.68 | 19 | 11 | 1.70 |
| sonnet5 | 3.00 | 14 | 16 | 1.40 |
| gpt54mini | 1.08 | 26 | 4 | 0.94 |
| glm53 | 1.17 | 18 | 12 | 0.70 |

### Reading it

**The wordless-turn correction is the finding, and it changes the answer.** A
turn that produces a bare tool call and no words leaves the user watching a
spinner with nothing behind it — and it records no TTFT at all, so it silently
vanishes from the latency column too. Sonnet 5 did that on **16 of 30** turns and
Kimi on 11; their fast-looking tails and their high quality scores are both
computed over the minority of turns they chose to speak on. Haiku did it on
**zero**. Scored across all thirty turns with silence counted as the failure it
is, Haiku comes first on quality; scored only where each model spoke, it looks
mid-table. The second number is the flattering one and the first is the true one.

**Haiku does not meet p90 ≤ 2.5 s, and the bar was the wrong bar.** Median 2.9 s,
p90 7.5 s; take off ~1.9 s of CLI startup that `/reply` does not pay per turn and
it is roughly 1.0 s median, 5.6 s p90. The tail is six turns between 7 and 12 s,
and every one of them is a turn where Haiku said a short sentence and *then*
called a tool. That target came from the voice bench, where a gap is dead air with
a person waiting out loud. In a chat window, a turn that has already printed
"Let me check your dashboard" and is visibly running a tool is not the same
failure. The bar is restated here as voice-only.

**deepseek-chat is the alternative, and it is a real one.** It is the only
candidate that clears p90 ≤ 2.5 s — 1.83 s median, 2.06 s p90, a 230 ms spread,
the tightest number in either bench — and it is second on corrected quality. It
costs 3.3x more per turn than Haiku (inflated by the same 0% cache bug) and went
wordless on four turns. If the p90 target is kept as written, deepseek-chat is
the model that meets it.

**Two candidates are disqualified on their own merits.** gpt-5.4-mini picked the
right tool on **1 of 10** tool turns — not a latency problem, a usefulness one.
GLM-5.3 is unusable for chat at a 14.7 s median and a 54.7 s p90, which is worth
knowing precisely because it is a reasonable choice for other jobs.

### Recommendation — chat

**`chat_provider: anthropic` / `chat_model: claude-haiku-4-5-20251001`.** First on
quality across all thirty turns, cheapest per turn by 3.3x, the only candidate
that never left the user staring at nothing, joint-best on tool choice, and the
only one besides Sonnet whose prompt cache works. It misses the p90 target, and
that target belongs to voice.

## What Jesse should set

Nothing in this PR changes a configured machine. The measured defaults apply only
where **nothing at all** is configured — an explicit `GOOSE_MODEL` outranks them
(see `crates/goose/src/config/model_roles.rs` for why chat and harness differ from
voice on this point). Jesse's `~/.permagent/config.yaml` was not touched.

To take the chat recommendation while leaving the coding harness exactly where it
is today, add to `~/.permagent/config.yaml`:

```yaml
chat_provider: anthropic
chat_model: claude-haiku-4-5-20251001

# Pin the harness explicitly so it does not follow GOOSE_MODEL around.
harness_provider: zai
harness_model: glm-5.3
```

Both keys of a pair or neither — half a pair is ignored with a warning. To take
this bench's harness recommendation as well, change the last two lines to
`anthropic` / `claude-haiku-4-5-20251001`. To put either job back on the session
model, set its model key to `session`.

## Shipped in this PR

- **`crates/goose/src/config/model_roles.rs`** — one module, one role enum, both
  defaults, and the precedence table as executable tests. Voice keeps its own
  module — deliberately, not as a leftover: `resolve_voice_model` puts the
  voice default ABOVE `GOOSE_MODEL`, while chat and harness defer to it, and
  collapsing the two resolvers would have to lose that difference.
- **The harness knob** (`harness_provider` / `harness_model`), read only for the
  coding recipe, wired into `resolve_provider_and_model`. The five-source
  precedence chain was extracted into `first_configured` so the ORDER itself is
  unit-tested rather than living in a chain of `.or_else` calls no test could
  reach.
- **The chat knob** (`chat_provider` / `chat_model`), applied by
  `crates/goose-server/src/chat_model.rs` on the `/reply` path and nowhere else.
  An unreachable route logs a warning and leaves the session on its existing
  model: a default that is wrong must never become a failed turn.
- **Settings → Models** grows a Chat / Voice / Harness table showing each job's
  effective model and where it came from (`from chat_model`, `from GOOSE_MODEL`,
  `built-in default`). All three rows are editable; #1116's standalone voice
  field folded into the table's Voice row rather than sitting beside it.
- **`permagent-eval` can actually run** — the `--recipe`/`-t` conflict fixed via a
  generated per-task recipe, with a test that fails if both flags ever return.
  Plus `--use-keyring`, ledger token/cache columns, `harness_log::scan`, a
  `--budget-usd` stop, and the ten fixture tasks.
- **`crates/goose-server/src/bin/chat_model_bench.rs`** — the `/reply`-path chat
  bench, shipped unrun. It needs one keychain approval; the moment someone grants
  it, `--run` replaces Part B's numbers with same-path ones.
- **Nothing in `formats::openai::get_usage`.** This branch carried a
  `prompt_cache_hit_tokens` fix until #1122 landed the same three-spelling
  mapping; the duplicate was dropped on rebase rather than re-litigated. Every
  0% cache-read figure in this document was measured BEFORE that landed, which
  is one more reason the harness recommendation asks to be re-run.

## Reproducing

```bash
# Harness: task-major sweep, hard stop on MEASURED spend.
python3 scripts/bench/model_defaults_sweep.py --dry-run     # tasks, candidates, projected row cost
python3 scripts/bench/model_defaults_sweep.py --budget-usd 10 \
        --tasks defaults-py-percentage-returns-a-fraction,defaults-ts-gold-discount-cap-not-enforced

# Chat, the way it should be run once the keychain dialog has been cleared once:
BENCH="cargo run --release -p permagent-daemon --bin chat_model_bench --"
$BENCH --probe      # which providers are configured (presence only, never a value)
$BENCH --dry-run    # projected spend, no requests
$BENCH --run --judge openai/gpt-5.4-mini --out chat_results.json
$BENCH --judge-only chat_results.json --judge openai/gpt-5.4-mini   # re-score, no new spend

# Chat, the way it was actually run here (signed CLI, no keychain dialog):
python3 scripts/bench/model_defaults_chat.py --budget-usd 5
```

Both refuse to start if projected spend exceeds the cap, refuse outright to spend
on a model with no canonical price, and stop the moment MEASURED spend reaches it.

## Not measured

- **Seven of the ten fixture tasks.** The spend stop refused the third six-way row
  at $7.66 of a $10 cap. They are in `crates/permagent-eval/tasks/`, verified, and
  cost about $3.60 per six-way row.
- **Chat on the daemon's own `/reply` path.** Blocked on a keychain dialog no
  unattended run can clear; measured through the signed CLI instead, with the
  difference stated above.
- **Cost on chat tool turns.** Refused before usage reaches the ledger, so `$/turn`
  covers the 20 non-tool turns only.
- **Anything after the prompt-cache fix.** Every 0% cache-read number here is a
  measurement of a bug as much as of a model.
- **Multi-turn chat.** Every bench turn is a single user message. Real turns carry
  history, so the absolute latencies are optimistic; the gaps are the finding.
- **MiniMax-M2.7, GLM-4.7 and deepseek-reasoner.** Probed and reachable, cut from
  the sweep for budget. Their absence is a budget decision, not a result.
- **Whether the ~17k-token capability inventory needs to be there at all.** It is
  76% of a prompt that is resent on every one of 20-odd calls per task, and it is
  the single largest number in this document. #1118.
