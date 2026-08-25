# Public suite: Aider polyglot through the Permagent coding harness

The public-facing companion to [`HARNESS_BENCHMARKS.md`](HARNESS_BENCHMARKS.md).
That file measures our own task suite, which nobody else runs. This one runs a
**recognised public suite**, so a number here can be put next to somebody
else's number — with the differences stated out loud rather than left for a
reader to discover.

Design, suite selection, and the hardware numbers behind it:
[`docs/research/HARNESS_PUBLIC_BENCHMARK_2026-08-25.md`](../research/HARNESS_PUBLIC_BENCHMARK_2026-08-25.md).

## How to reproduce a row

```sh
git clone https://github.com/Aider-AI/polyglot-benchmark   # pin the SHA in the row
scripts/bench/polyglot_bench.py prepare --suite <clone> --workdir <dir> \
    --lang python --n 10 --seed 20260825
scripts/bench/polyglot_bench.py run   --workdir <dir> --label <label> \
    --provider <p> --model <m> --max-turns 30
scripts/bench/polyglot_bench.py grade --workdir <dir> --label <label>
scripts/bench/polyglot_bench.py report --workdir <dir>
```

Selection is `sorted(random.Random(seed).sample(sorted(names), n))`, so the
same seed against the same suite SHA yields the same task list anywhere.

What each phase guarantees:

- **prepare** deletes `.meta/` (the reference solution) from the agent's
  workspace and keeps a pristine copy the agent never sees.
- **run** appends the task as a `prompt:` block to a *verbatim* copy of the
  shipped `permagent-coding.yaml` and records both files' sha256, because
  `--recipe` and `--text` cannot be combined and a silently-edited recipe
  would invalidate everything.
- **grade** copies only the agent's solution file(s) into the pristine
  exercise and runs the suite's own tests there. Weakening a test buys
  nothing; the tamper is recorded in its own `test_files_tampered` field.
- Cost, tokens, calls and the **model that was actually billed** come from the
  runtime's `cost_ledger` table, never from a model's self-report. An
  unmeasured field is `null`, not `0`.

## Leaderboard

Keyed by (date, suite SHA, harness commit, model). All four move
independently; a row missing any one of them is not reproducible.

### 2026-08-25 — PILOT, n=10 — Aider polyglot Python subset

> **PILOT — n=10. Not a benchmark result.** One task is ten percentage
> points. The gap between the two rows below (90% vs 100%) is one task, and
> that one task is a coin-flip at this sample size. This run exists to prove
> the measurement path works and to surface what it measures; it does not
> rank two models.

- suite `Aider-AI/polyglot-benchmark` @ `7e0611e77b54e2dea774cdc0aa00cf9f7ed6144f`
- harness commit `9679fafa`, recipe `crates/goose-cli/src/recipes/builtin/permagent-coding.yaml`
- seed `20260825`, 10 of the 34 Python exercises, `--max-turns 30`, run sequentially on the M4
- graded with `python3 -m unittest discover -p "*_test.py"` (CPython 3.14.7) in a pristine copy

| model billed | pass@1 | cost | $/task | wall | calls | tool calls | cache read | no code | tamper | 429s |
|---|---|---|---|---|---|---|---|---|---|---|
| `custom_deepseek/deepseek-v4-flash` | 9/10 | $1.61 | $0.161 | 8m18s | 104 | 97 | **0%** | 1 | 0 | 0 |
| `anthropic/claude-haiku-4-5-20251001` | 10/10 | $2.35 | $0.235 | 6m49s | 151 | 136 | **70%** | 0 | 0 | 0 |

Per-task detail and the raw JSON records: `scripts/bench/results/polyglot/`.

#### What the pilot found that the pass rate does not say

**1. `deepseek-chat` is not what gets billed.** The ledger says every call went
to `deepseek-v4-flash`. The alias resolves elsewhere, and no self-report would
have mentioned it. Any row on this page that named only the requested model
would have been wrong.

**2. Zero prompt caching on the DeepSeek path; 70% on the Anthropic one.**
DeepSeek: 3,507,240 input tokens, 0 cache-read, 0 cache-write, across 104
calls — a floor of ~24.6k input tokens *per call*, which is the fixed system
prompt and repo map being re-billed every single time. Anthropic: 3,394,720 of
4,837,180 input tokens served from cache. Same harness, same tasks, same day.
This is a provider-path gap in the harness, not a property of either model,
and it is worth real money: roughly 2.5M of DeepSeek's 3.5M input tokens were
the same preamble sent 104 times.

**3. Input dwarfs output by 69:1.** 3.5M in against 51k out for ten exercises
that are each a few dozen lines of Python. The harness's context, not the
model's thinking, is what these runs cost.

**4. Exit code 0 is not a verdict.** DeepSeek's `bowling` run exited cleanly
after 4 tool calls and 71 seconds having written *nothing* — the solution file
was byte-identical to the stub, and all 31 tests failed. The model reasoned
about the algorithm in prose and never called the edit tool. This is why the
grader diffs against the stub and reports `no code written` as its own column:
"wrote wrong code" and "wrote no code" are different failures and must not
read the same in a table.

**5. The recipe's mandated independent reviewer never fired.** The
`permagent-coding` recipe instructs the agent to summon a cross-model reviewer
before declaring non-trivial work done. Subagent cost across all 20 runs:
$0.00. Either these tasks read as trivial to the agent, or the instruction is
not landing. Worth a look — it is one of the harness's advertised safeguards.

#### What a full run would cost

Extrapolating the measured $/task and wall time, and saying plainly that the
Python subset is not representative of the other five languages:

| | 225-task full polyglot | note |
|---|---|---|
| `deepseek-v4-flash` | ~$36, ~3.1h sequential | at $0.161/task, 50s/task |
| `claude-haiku-4-5` | ~$53, ~2.6h sequential | at $0.235/task, 41s/task |

For scale: Aider's own leaderboard lists its most expensive entry, GPT-5
(high), at **$29.08** for the same 225 tasks. This harness on the *cheapest*
Anthropic model would cost roughly twice that. Most of the difference is
finding 2 and finding 3 — the harness's own context, re-sent. That is the
number the harness should be trying to move.

No SWE-bench Verified estimate is offered. These are single-file Exercism
exercises; SWE-bench instances are real repositories, and extrapolating
per-task token cost from one to the other would be a guess wearing a
number's clothes. The blocker there is hardware, not budget — see the design
doc.

## Not directly comparable to the published Aider leaderboard

The [Aider leaderboard](https://aider.chat/docs/leaderboards/) (last updated
2025-11-20; GPT-5 high at 88.0%, $29.08 for a full run) is a different
measurement, in four ways:

1. **Different task set** — 225 exercises across six languages there; a seeded
   subset of the 34 Python exercises here. Python was chosen because its
   Exercism tests are all stdlib `unittest`, so grading needs no Docker, no
   network and no per-task install; the other five languages need a JDK, Go,
   `npm install`, or `cargo fetch` per exercise and are not wired up.
2. **Different protocol** — Aider allows two attempts, the second with the
   test output fed back once. This harness runs its own agentic loop with a
   verify tool and a retry budget bounded only by `--max-turns`. More
   iterations are available here than the leaderboard allows.
3. **Different scaffold** — Aider edits through search/replace blocks with no
   shell. This harness has a shell, a repo map, structured search, and a
   cross-model reviewer. What is measured is the harness at least as much as
   the model.
4. **Contamination is unbounded** — these are public Exercism exercises with
   public solutions, frozen since 2024-12-22, and every model tested was
   trained after that. Treat the absolute number as an upper bound on
   capability. It is only meaningful as a *relative* comparison between models
   run on the identical task list, through the identical harness, on the same
   day.

## What is not on this page

- No SWE-bench number of any kind. SWE-bench's harness asks for x86_64,
  120 GB free disk, 16 GB RAM and 8 CPU cores; the only Docker host here is an
  aarch64 colima VM with 2 CPUs, 4 GiB RAM and 15 GiB free on its host. See
  §2 of the design doc for what would have to change.
- No claim of the form "the harness scores X on the Aider polyglot benchmark."
  The honest sentence is "the harness solved N of M Python exercises from the
  Aider polyglot suite, at seed S, on date D."
- No result at n<30 without PILOT beside it. At n=10 one task is ten
  percentage points, and the interval is wider than any gap the table shows.
