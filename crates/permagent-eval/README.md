# permagent-eval

A lightweight, objective eval harness for the **Permagent coding harness**
(`permagent run --recipe permagent-coding`). It runs a curated set of coding
tasks, grades each with a deterministic test, reads the run's dollar cost from the
per-call cost ledger (#714), and reports **pass-rate, $/solved, and median
$/task** — runnable under different model tiers so you can compare a cheap/local
model against a frontier one.

This is deliberately *not* the full 225-problem Aider-polyglot Docker
orchestration. It is a small curated set (currently 7 tasks) that answers one
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
| `minicpm5` | `ollama` | `openbmb/minicpm5:q8_0` | nothing (on-device, $0); 1200s harness-timeout floor |
| `minicpm5-hf` | `ollama` | `minicpm5-2b-q8` | nothing (on-device, $0); fallback if the `minicpm5` Ollama tag mismatches the HF GGUF — build via a local Modelfile; 1200s harness-timeout floor |

Or define an ad-hoc tier with `--provider <id> --model <name>`.

### Harness wall-clock ceiling (Lane K6 / K6-b)

Each task run is bounded by a wall-clock ceiling (`harness_timeout_secs`,
default 900s) — exceed it and the harness is killed and the task counts as
timed out. The ceiling is resolved per run:

1. `--harness-timeout-secs <N>` on `run`/`ab` — a deliberate measurement
   knob for ONE invocation, overriding everything below (even a tier's
   floor).
2. Otherwise, if the **tier** declares its own `harness_timeout_secs`
   (most tiers have none; `glm47`/`glm53` — the zai tiers — default to
   900s), the ceiling is `max(task cap or 900s default, tier value)` — the
   tier value is a FLOOR, never a shortening override, so it raises the
   ceiling for a slow provider without ever cutting a task's own cap short.
3. Otherwise, the task's own `harness_timeout_secs:` in its `task.yaml`, or
   the driver's default (900s) if it has none.

Lane K6-b's rationale: a fixture's `harness_timeout_secs:` encodes "a
competent harness finishes within N s on a fast model" — it's a
per-fixture budget, not a claim about every provider's speed. zai's own
solved-task walls on this suite run 150-440s (3-5x DeepSeek's 40-200s),
so two fixtures whose task-level cap was tuned to 600s for faster tiers
now resolve to `max(600, 900) = 900` under the zai tiers' floor, with no
CLI override needed.

The resolved value is retained per task as `harness_timeout_secs` in the
report/`.qual.json` record, so latency evidence can always be read against
the ceiling that actually produced it.

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
comparison table (a repeated tier is de-duplicated with a warning). Use
`--task <id>` to run a subset, `--keep` to keep the scratch dirs for debugging,
and `--native-routing` to measure the shipped cost-optimizer instead of a pinned
single model. Add `--fail-under <percent>` to make the process exit non-zero
unless at least one tier reaches that pass-rate (for CI gating); without it the
exit code is always 0.

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

### Oracle import discipline (tamper-proofing)

The oracle overlay only re-installs files that share a name with a pristine
oracle file. But an oracle runs **inside the finished workspace**, and for
`python3 <script>` the interpreter puts the script's own directory (the
workspace) first on `sys.path` (Node resolves bare specifiers there too). So any
module an oracle imports that the agent could have written is a shadowing hole: a
hostile solution can drop a `json.py` (or `statistics.py`, …) that feeds the
grader rigged data and flips a real FAIL into a PASS.

To make this a rule rather than a matter of authorship luck, `validate` (and
every task load) enforces:

- **Python oracles** may import only `sys` (a built-in — compiled into the
  interpreter, so unshadowable) plus the task's declared **`deliverables`** — the
  workspace modules the agent is meant to produce (e.g. `deliverables: [stats]`
  for a `from stats import median` grader; note the module name can differ from
  the task id, e.g. `roman` for `roman-numerals`). Any other bare
  `import X` / `from X import …` is rejected.
- **Node oracles** may use only `node:`-prefixed specifiers in static
  `import … from "…"`, `require("…")` or `import("…")`. Load a deliverable by an
  absolute `file://` URL —
  `import(pathToFileURL(process.cwd() + "/merge.mjs").href)` — which is not a
  shadowable bare specifier.

A task whose oracle imports an undeclared, workspace-shadowable module fails
`validate` (and never runs), so the grade can't be rigged by a planted module.

## SWE-bench slice (SWE-arm-30)

`benchmarks/swe-arm-30.yaml` pins 30 SWE-bench Verified instances (plus 6
spares) for the fast paired A/B loop: dataset
`SWE-bench/SWE-bench_Verified@78f471bf655a3137b2e8a75af1501690ec009ec3`
(the copy that ships each instance's `eval_script` and `log_parser`,
cross-checked row by row against `princeton-nlp/SWE-bench_Verified@c104f840…`),
Verified-mini members first, stratified 12 "<15 min fix" / 18 "15 min - 1 hour",
capped at 8 per repo, no scikit-learn / matplotlib / xarray, and only instances
whose Epoch arm64 image (`ghcr.io/epoch-research/swe-bench.eval.arm64.<id>`)
exists — pinned by digest. `benchmarks/swe-arm-30.instances.jsonl` holds what
the judge needs (issue text, eval script, test lists); gold patches are never
stored. `benchmarks/tools/select_swe_arm30.py` reproduces the selection and
`benchmarks/tools/validate_swe_arm.py` checks it (`--static`: registry only;
`--remote m1`: gold twice + empty once on the M1, disk-floor guarded).

A SWE-bench instance becomes an ordinary task, so it runs through the same
tiers, cost ledger, report and `--record`/`qualify` path as the curated set:

```bash
# 1. Materialize: repo at base_commit as workspace/, issue as the prompt, and
#    an oracle that calls back into `swe-judge`. Needs GitHub access (blob-less
#    mirrors are cached under ~/Library/Caches/permagent-eval/swe-repos).
cargo build --release -p permagent-eval
./target/release/permagent-eval swe-materialize \
  --slice crates/permagent-eval/benchmarks/swe-arm-30.yaml \
  --out ~/swe-arm-30-tasks --ssh m1
#    (--instance <id> for a subset, --spares to add the spares,
#     --harness-timeout-secs/--max-turns to bound each run)

# 2. Run it like any task set.
./target/release/permagent-eval run --tasks-dir ~/swe-arm-30-tasks \
  --tier <tier> --permagent-bin ./target/release/permagent \
  --format json --out swe-arm-30.json

# 3. Per-task cost, model calls and wall time, and $/solved.
./target/release/permagent-eval swe-summary --report swe-arm-30.json \
  --slice crates/permagent-eval/benchmarks/swe-arm-30.yaml \
  --verdict-dir ~/swe-arm-30-tasks/verdicts
```

**Judging.** Each task's oracle is `permagent-eval swe-judge --slice … --instance
<id> …`, run in the finished workspace. It diffs the workspace against the
runner's seed commit (untracked files included, commits the harness landed
included, `.permagent*`/`.goose` excluded), then on the Docker host over SSH:
pulls `image@digest` only if absent and only if the host keeps
`--min-free-gib` (default 15) free, pipes the patch and the dataset's own
`eval_script` into `docker run -i --memory 2g … bash -s` (patch applied with
`git apply`, then `patch --fuzz=5`, as upstream), grades the log with ports of
the upstream parsers (django, sympy, pytest and its options/v2 variants,
seaborn), then removes the container, removes the image if it pulled it, and
`fstrim`s the Colima disk so the host gets the space back. **Resolved** = every
`FAIL_TO_PASS` test passes and every `PASS_TO_PASS` test still passes (skipped
counts as maintained for P2P only). Exit 0 = resolved, 1 = not resolved,
2 = judge/infrastructure error (disk floor, pull, SSH, Docker). The runner
counts 1 and 2 alike as a fail; each verdict (and the judged patch) is written
to `--verdict-dir`, which `swe-summary` uses to show judge errors separately.
Judge a patch directly with `swe-judge --patch-file gold.diff` or `--empty`
(validation).

**Cost, turns, time.** Cost is the run's isolated ledger total (as for every
task); "model calls" is the ledger's row count (one per provider response,
sub-agents included); wall time is harness plus judge. `$/solved` is total
spend over every attempted instance divided by instances resolved.

**Caveats.** Epoch's arm64 images are best-effort and upstream calls them
untested, so every instance must pass gold twice and fail empty before its
results count; see the slice file's `validation` block for what has been
checked. The agent's local checkout has no test environment (the tests run
only in the image). The oracle argv embeds the absolute path of the binary
that ran `swe-materialize` (override with `--judge-bin`); rebuild then
re-materialize if it moves.

## Tests

The pure logic — task loading/validation, invocation construction, oracle parsing,
cost aggregation from ledger rows, and the pass-rate/$/solved/median math — is
unit-tested and gated in CI (`cargo test -p permagent-eval`). The subprocess and
SQLite-reading glue sits behind traits so it is exercised with mocks; the real
cost reader is tested against a temporary ledger database. Running the actual
harness is not part of the unit tests.
