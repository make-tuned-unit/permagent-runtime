# permagent-eval

A lightweight, objective eval harness for the **Permagent coding harness**
(`permagent run --recipe permagent-coding`). It runs a curated set of coding
tasks, grades each with a deterministic test, reads the run's dollar cost from the
per-call cost ledger (#714), and reports **pass-rate, $/solved, and median
$/task** — runnable under different model tiers so you can compare a cheap/local
model against a frontier one.

This is deliberately *not* the full 225-problem Aider-polyglot Docker
orchestration. It is a small curated set (currently 6 tasks) that answers one
question: **how well, and at what cost, does the coding harness do real work on a
cheaper model?**

## How it works

For each (task, tier) the runner:

1. Copies the task's `workspace/` seed into a throwaway working directory.
2. Gives the run its **own isolated `PERMAGENT_PATH_ROOT`**, so the session +
   cost-ledger database contain exactly this one run (parent agent plus any
   sub-agents it summons) with no cross-run contamination.
3. Invokes the harness headlessly:

   ```
   permagent run --recipe permagent-coding -t "<task prompt>" \
     --provider <p> --model <m> --output-format text
   ```

   with `GOOSE_MODE=auto` (unattended tools) and `PERMAGENT_DISABLE_KEYRING=1`
   (read provider keys from the environment, not the OS keychain).
4. Overlays the task's pristine `oracle/` files onto the workspace (so the agent
   can't have weakened its own grader) and runs the task's `test` command. Exit 0
   = solved.
5. Reads total cost as `SUM(cost_usd)` over that run's isolated `cost_ledger`
   table. `$0.00` means genuinely free (local/Ollama); *unknown* (no ledger rows)
   is reported separately and never conflated with free.

Then it aggregates pass-rate, $/solved (total spend over all tasks ÷ tasks
solved — failures are amortised into the price of a solve), and median $/task.

## Model tiers

A **tier** pins the harness to one provider+model via `--provider`/`--model`, and
(unless `--native-routing`) pins the cost-router packs (`PERMAGENT_PACK_*`, #720)
to the same model so *all* work stays on the tier under test — an apples-to-apples
measurement of one model rather than the shipped cross-tier optimizer.

Built-in tiers:

| tier | provider | model | needs |
|------|----------|-------|-------|
| `local` | `ollama` | `qwen3` | nothing (on-device, $0) |
| `kimi` | `moonshot` | `kimi-k2.5` | `MOONSHOT_API_KEY` |
| `minimax` | `minimax` | `MiniMax-M2.5` | `MINIMAX_API_KEY` |
| `sonnet` | `anthropic` | `claude-sonnet-5` | `ANTHROPIC_API_KEY` |
| `frontier` | `anthropic` | `claude-opus-4-8` | `ANTHROPIC_API_KEY` |

Or define an ad-hoc tier with `--provider <id> --model <name>`.

## Running it (on the mac mini)

The eval must run where the app, the models, and the provider keys live — the mac
mini. From the repo root:

```bash
# 1. Build/install the permagent CLI so `permagent run` works, e.g.:
cargo build --release -p permagent-cli    # binary: target/release/permagent
#    (make sure Ollama is running with the local model pulled, e.g. `ollama pull qwen3`)

# 2. Set provider keys for any hosted tiers you want to compare:
export ANTHROPIC_API_KEY=...   MINIMAX_API_KEY=...   MOONSHOT_API_KEY=...

# 3. Sanity-check the task set (no models called):
cargo run -p permagent-eval -- list
cargo run -p permagent-eval -- validate
cargo run -p permagent-eval -- plan --tier local        # prints the exact commands

# 4. Run the eval. Point --permagent-bin at the binary from step 1:
cargo run -p permagent-eval -- run \
  --tier local --tier frontier \
  --permagent-bin ./target/release/permagent \
  --format md --out eval-report.md
```

`run` prints a live PASS/FAIL line per task to stderr and the full report to
stdout (and to `--out`). Pass multiple `--tier` flags to get a side-by-side
comparison table. Use `--task <id>` to run a subset, `--keep` to keep the scratch
dirs for debugging, and `--native-routing` to measure the shipped cost-optimizer
instead of a pinned single model.

### Reading the report

```
| tier     | pass-rate | solved | $/solved | median $/task | total $ |
|----------|-----------|--------|----------|---------------|---------|
| local    | 66.7%     | 4/6    | $0.0000  | $0.0000       | $0.0000 |
| frontier | 100.0%    | 6/6    | $0.0180  | $0.0026       | $0.1080 |
```

Local is (near) free, so its story is pass-rate. For hosted tiers, $/solved is the
cheap-vs-frontier number: how many dollars it costs to actually get a task solved.

## The task set

Under `tasks/<id>/`: `task.yaml` (spec + prompt + `test` argv), an optional
`workspace/` seed the agent sees, and a hidden `oracle/` that is overlaid before
grading. Current tasks:

- `tic-tac-toe` — playable HTML + a pure, tested `checkWinner` (game/UI)
- `merge-row-2048` — the 2048 slide-and-merge mechanic + HTML (game/UI)
- `fizzbuzz`, `palindrome`, `roman-numerals` — classic functions
- `scoring` — implement a scoring function to pass a provided test
- `fix-median` — fix a bug so the provided test passes

Oracles need `node` and `python3` on PATH (both present on the mini). Adding a task
is just a new directory following the same layout.

## Tests

The pure logic — task loading/validation, invocation construction, oracle parsing,
cost aggregation from ledger rows, and the pass-rate/$/solved/median math — is
unit-tested and gated in CI (`cargo test -p permagent-eval`). The subprocess and
SQLite-reading glue sits behind traits so it is exercised with mocks; the real
cost reader is tested against a temporary ledger database. Running the actual
harness is not part of the unit tests.
