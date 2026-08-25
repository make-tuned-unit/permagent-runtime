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

<!-- RESULTS -->

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
