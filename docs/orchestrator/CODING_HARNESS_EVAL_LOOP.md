# Coding-harness evaluation loop

This document defines the bounded experiment loop implemented by
`crates/permagent-eval/src/iteration.rs`. It is the measurement/training lane
for the coding-harness DAG; it does not encode a new model, copy another
agent's orchestration, or call a provider by itself.

## Contract

The loop receives one frozen task slice and a runner with two arms:

```text
for iteration in 0..max_iterations
  for repetition in 0..repetitions_per_iteration
    for task in frozen_tasks (stable caller order)
      control (feature off)
      treatment (feature on)
```

The runner is the only model-dependent seam (`ArmRunner`). A production adapter
can call the existing `run_task`/oracle/cost seams and attach a structured
`DagEvidence` receipt. Unit tests use a closure, so `cargo test -p permagent-eval`
never starts a model, provider CLI, or network request.

Every cell records:

- arm, task id, and repetition;
- deterministic oracle result (`Pass`, `Fail`, `Errored`, or `NotRun`);
- wall-clock seconds and harness signals already present on `TaskResult`;
- exact/estimated USD plus input/output/cache token totals;
- structured DAG validity, bounds, route, verification, review/landing gates,
  node counts, and mutation-boundary violations.

## Bounds and uncertainty

`IterationConfig` requires positive `repetitions_per_iteration`,
`max_total_runs`, and `max_iterations`. A shared `budget_usd` is charged only
from measured ledger USD; an unknown reading contributes `$0` to stopping but is
still surfaced in the aggregate and can be made a graduation failure with
`require_known_costs`. Once either cap is reached, every remaining cell is
explicitly `NotRun`, never `Fail`, and is excluded from pass-rate and paired
delta.

The fixed task order is intentional: task IDs and dataset revisions are frozen
outside this crate (see `huggingface-benchmarks.yaml`). Any external importer
must resolve immutable revisions and persist the selected IDs before invoking
the loop. Do not use a moving benchmark `main` as a treatment.

## Graduation gates

`GraduationGates` defaults to the conservative policy:

- at least 30 attempted runs per arm (the paired module's confidence floor);
- the treatment arm at least 80% pass-rate (the control is a paired baseline,
  so a weak control does not prevent a demonstrably better treatment from
  graduating);
- treatment delta must not regress control;
- every attempted run must carry fully compliant DAG evidence;
- three consecutive qualifying snapshots before promotion.

Optional latency, `$`/solved, known-cost, and token-availability gates can be
enabled for a release qualification. Relative token and cost-reduction gates
can require the treatment to improve efficiency against the control. A negative treatment delta is `Rejected`;
insufficient sample, missing evidence, or an unmet efficiency bar is `Hold`.
Only `Graduated` is permission to promote the treatment. A successful task
without a compliant DAG receipt is therefore useful diagnostic data, not a
promotion signal.

## Iteration policy

Use the loop as a training/repair feedback cycle:

0. **M0.5 -- replay slice green before any paid slice.** Before spending
   anything, run `cargo test -p permagent --lib -j1 -- --ignored
   replay_slice_smoke` (or, once a real cassette is recorded, `permagent-eval
   run --cassette <dir> --cassette-mode replay`). See "M0.5" below.
1. Run a small smoke slice with a hard run and dollar cap.
2. Inspect the report's first failed gate and per-task evidence.
3. Change one harness idea (for example routing, recovery, or verification).
4. Re-run the identical frozen slice as control/treatment.
5. Increase repetitions only after wiring and DAG compliance are clean.
6. Promote only on `Graduated`; otherwise retain the report and reason text.

This keeps improvements attributable and prevents a larger benchmark from
masking a broken control-plane gate. Public benchmark slices remain staged and
budget-gated; the 17 bundled Permagent tasks are the fast deterministic wiring
gate.

## Exceptional-state defect graph

This loop overlays **every** child node in the coding-harness master program.
It is not a final cleanup pass and it is not another scheduler. Runtime work
continues to use the approved Permagent roadmap; Spectral/session storage
remains the source of project memory. The graph below defines how evidence from
that work is converted into a bounded repair and qualification decision.

```text
M0 Observe a frozen baseline
 |  missing signal / cannot reproduce
 |<---------------------------------------------+
 v                                              |
M1 Classify defect + severity + owning DAG node |
 |                                              |
 v                                              |
M2 Pin the failure with a deterministic test ---+
 |
 v
M3 Apply the smallest causal repair <---------------------+
 |                                                        |
 v                                                        |
M4 Focused verification -- fail / changed diagnosis ------+
 |
 v
M5 Integrated invariants -- regression -> M1 (new defect)
 |
 v
M6 Paired held-out retest -- regression -> reopen owning node
 |
 v
M7 Retain evidence and candidate
 |  fewer than 3 consecutive qualifying snapshots
 +---------------------------> M0
 |
 |  3 qualifying snapshots; every area Excellent
 v
Promotion / release gate
```

### Defect lifecycle contract

Anything that makes the harness less correct, trustworthy, efficient, or
inspectable is a defect. This includes failed assertions, false-positive
success receipts, swallowed child failures, unnecessary retries, duplicate
effects, stale or missing evidence, context loss, avoidable cost, latency
regressions, misleading provider/model attribution, and verification that is
disproportionate to risk.

A defect is closed only when one retained evidence bundle contains:

1. the observed symptom and owning master/child DAG node;
2. a stable reproducer or, when reproduction is impossible, the new
   instrumentation that will make a recurrence diagnosable;
3. the causal hypothesis and smallest repair attempted;
4. focused verification for the repaired behavior;
5. integrated verification for adjacent trust, accounting, routing, memory,
   and recovery invariants;
6. a paired held-out result proving the repair was retained rather than merely
   fitted to the reproducer.

An unchanged failed command is never repeated. A retry requires at least one
of: changed implementation, changed diagnostic, changed environment with a
recorded reason, or a distinct recovery action. Attempt, time, token, and dollar
caps remain hard stops. A P0/P1, authorization boundary, or spend-cap crossing
is routed to the existing approval system; ordinary deterministic success does
not manufacture a human gate.

### M0.5 -- replay slice green before any paid slice

A mechanism regression (a rejected approval payload, an orphaning requeue, a
misfired dead-worker detector) does not need a live model to reproduce -- it
lives entirely in the coordinator/orchestrator's own control-plane code. Every
M0-M7 pass before Lane R1 paid a live provider to (re)discover this class of
defect, at $0.05-$0.27/task and 2-8 minutes per fixture (M8-M11 in this log).
That cost buys nothing for a defect that would have reproduced against a
canned model response just as well.

`crates/goose/src/providers/cassette.rs` (Lane R1) adds a `Provider` decorator
installed at the single factory choke point (`provider_registry.rs`'s
`ProviderEntry::create`/`create_with_default_model` -- the same choke point
`SovereignGuardProvider` uses), gated by one env var:

- `PERMAGENT_PROVIDER_CASSETTE=record:<dir>` -- records a real run's provider
  traffic once, as JSONL, alongside the real call (never changes what the
  caller sees).
- `PERMAGENT_PROVIDER_CASSETTE=replay:<dir>` -- replays it. No real provider
  is constructed at all (no credentials, no network, no spend). A request
  that does not match a recorded one fails loudly (`<dir>/misses.jsonl`) --
  a cassette never fabricates a response. The recorded provider's cost tier
  and data locality are replayed too, so a recorded PAID call still goes
  through the same reservation/budget/approval mechanics a real paid run
  would (a cassette with no recorded tier fails closed to paid/cloud).

`permagent-eval run --cassette <dir> [--cassette-mode record|replay]` rides
the existing A/B-arm environment seam (`RunConfig::arm_env`, applied last)
to set this on the child `permagent` process -- no new plumbing in
`runner.rs`/`invocation.rs`. A replay run also waives the API-key preflight
(`--cassette-mode replay` only): nothing is spent, so nothing needs a
credential. A `#[ignore]`d library test, `replay_slice_smoke` (`cargo test -p
permagent --lib -j1 -- --ignored replay_slice_smoke`), records one real
`handle_decompose_roadmap` call against this file's own deterministic local
fixture provider and then REPLAYS it through a fresh session, DB and client
that contain no provider but the cassette -- asserting byte-identical
proposed goals, an identical roadmap content hash, a real `choice` approval
decision on both legs, and that root-goal dispatch still fires. In-process,
on the same test binary the gates already compile -- no separate build, no
signing, no network. The factory wiring itself (`replay:` never constructing
a real provider; `record:` still constructing and sovereignty-guarding one)
is pinned separately and deterministically by `provider_registry.rs`'s own
unit tests, which need no env var and no process globals.

Run `replay_slice_smoke` by that exact filter. It installs the process-global
`GOAL_DISPATCH_HOOK` (a `OnceLock`), so it is only reliable in a test process
where nothing else has claimed that seam -- which the `--ignored
replay_slice_smoke` filter guarantees, and a `--include-ignored` full-suite
run does not.

**What this does and does not catch.** A replay slice proves the
control-plane mechanism (decompose/roadmap/approval/dispatch/verify/
review/landing state machine) behaves correctly against a FIXED sequence of
model responses. It cannot catch a defect that only manifests with a
DIFFERENT model response than the one recorded (a new model quirk, a
different tool-call shape) -- that is still M0-M7's job, on a live slice.
Treat a green replay slice as "the mechanism didn't regress," not as "the
model will solve this fixture" -- those are different claims and this loop
only ever answers the first for free.

**Recording a real cassette (M12's job, not this lane's).** Run a normal
`permagent-eval run --cassette <dir> --cassette-mode record --tier <tier>
--task <id>` against a real fixture and tier once; commit the resulting
`<dir>/cassette.jsonl` (secrets are stripped before it is written -- see the
module doc in `cassette.rs` for exactly what is and is not stripped, and its
documented limitations). From then on, `--cassette-mode replay` reproduces
that slice for $0 until the harness or the fixture changes shape enough that
the cassette's keys stop matching -- at which point it fails loud
(`misses.jsonl`), never silently wrong, and needs re-recording.

### Measurement surface

Every iteration scores correctness, trust and safety, DAG validity, context and
Spectral-memory fidelity, provider/model routing, fault recovery, USD per solved
task, token use, latency, tool efficiency, code-pattern conformance, and
verification efficiency. Unknown or missing evidence is `Unrated`, never a
pass. Any integrated or held-out regression opens a new defect and reactivates
the earliest owning DAG node before downstream qualification can continue.

The master program's `p7b_continual_defect_loop` qualifies this feedback
mechanism itself after the functional child DAGs pass. It does not defer defect
handling until P7: M0-M7 applies to P0 through release, and P7b proves that the
loop closes and reopens defects correctly before held-out qualification.

The controller back-edge is executable and atomic:

```sh
permagent-eval program reopen \
  --manifest docs/orchestrator/CODING_HARNESS_MASTER_PROGRAM_DAG.yaml \
  --node p4_task_budget_boundary \
  --reason "retained evidence: <artifact or run id>" \
  --in-place
```

Without `--in-place` or `--out`, the command is a read-only projection. It
reactivates an approval-free owner immediately, resets every downstream
descendant to `planned`, and never bypasses a human or spend-cap approval.

## M0–M4 pass, 2026-09-06: three defects found by running the harness

Method per the iteration policy: a small slice, hard caps, free local arm, then
inspect the first failure rather than enlarge the sample. Runner
`permagent-eval run --provider ollama --model qwen2.5:7b`, tasks `fizzbuzz` and
`fix-median`, `--keep` so the scratch survives. `qwen2.5:7b` was used because the
built-in `local` tier pins `qwen3`, which is not installed on this machine; no
model was downloaded for this pass. Cost of the whole investigation: $0.00.

### M0 baseline against the installed Sep-3 CLI

0/2 solved. `fizzbuzz` spent its full 25-turn budget and 222s; `fix-median` 24
turns and 151s. The scratch workspace was the evidence: `fizzbuzz.py` existed and
was **zero bytes**.

### M1 classification, from the retained harness log

The turn-by-turn log shows the model wrote a **correct** solution on its first
tool call, overwrote it with a wrong shape on the second, then wrote empty
content twice. Three harness defects, each independent of model strength:

1. **A truncating write is reported in ordinary success language.** The third
   write emptied a file holding a working solution and the harness answered
   `Wrote fizzbuzz.py (0 lines, verified on disk)`. "Verified on disk" is
   literally true — the emptiness was verified — and it reads as an
   accomplishment. Nothing said that the answer the model had already produced
   was now gone, and it spent every remaining turn verifying an empty file. This
   is the false-positive-receipt class named in the defect lifecycle contract.
2. **An empty `path` reached the filesystem.** `path: ""` resolved to the working
   directory and returned `Failed to write : Is a directory (os error 21)` — an
   error about a directory the model never named. It then wrote itself a long
   explanation of a path bug it did not have. A prior call with `path` absent
   leaked the raw serde error `Failed to parse arguments: missing field 'path'`.
3. **`ToolMissing` named the shell instead of the missing program.** `verify`
   reported "`shell` was not found on PATH, so `npm run test` could not run.
   Install it" — `Check::program_name()` returns the literal `"shell"` for any
   `Exec::Shell`. There is no program called `shell`, so the advice was
   unfollowable; the missing program was `npm`.

### M2/M3 repairs, each pinned by a test

- `developer/edit.rs`: a write whose content is empty over a non-empty file now
  says it replaced N bytes and that the work is gone. It stays a `success` —
  clearing a file is legal — but it cannot be mistaken for an ordinary write.
  Deliberately creating a new empty file keeps the quiet message.
- `developer/edit.rs`: an empty or whitespace-only `path` is refused before the
  filesystem is touched, naming the `path` argument.
- `developer/verify.rs`: `program_name()` returns the first token of a shell
  command line, so the message names `npm`/`cargo`.

Tests: `write_names_the_work_an_empty_content_erased`,
`write_keeps_its_ordinary_message_when_creating_an_empty_file`,
`write_refuses_an_empty_path_instead_of_writing_the_directory`,
`shell_check_names_the_missing_program_not_the_shell`. The reproducer for the
first three is the retained log quoted above — a production observation, not a
synthetic revert. `developer::edit` 45 passed, `developer::verify` 44 passed.

### The regression this exposed: free local inference was refused

Re-running the identical slice against a locally built CLI instead of the
installed one produced **0 tool calls in 2.1s** and this refusal:

```text
I did not send this request because provider spend could not be authorized:
provider budget is unknown; refusing call: cannot authorize provider call:
context limit is required for a paid reservation.
```

Ollama has no marginal cost. `OllamaProvider::cost_tier()` correctly returns
`LocalFree`, and `plan_reservation_bound` correctly returns `Ok(None)` for a
non-chargeable tier — but `SovereignGuardProvider` wraps every provider at the
single factory choke point and did **not** override `cost_tier()`, so it answered
the `Provider` trait's fail-closed `PaidApi` default on Ollama's behalf. A free
call then needed a paid reservation and was refused for want of a context limit.

`sovereign_guard.rs` already carries a comment about this exact class for
`stream_split` — "a decorator quietly disabling a property of the thing it
decorates". The same slip had been made for cost attribution. Repaired by
delegating `cost_tier` to the inner provider, pinned by
`wrap_preserves_free_cost_attribution`; `sovereign_guard` 6 passed.
`get_initial_user_messages` is also undelegated but its trait default is generic
and nobody overrides it, so it was deliberately left alone.

After the repair the identical slice runs again: 9 tool calls, cost correctly
reported as `$0.0000` rather than unknown.

### Honest limits of this pass

Pass-rate is **0/2 before and after**. Nothing here claims a quality improvement:
a 7B local model failing FizzBuzz is not evidence about the harness, and the
graduation gates were not run — no paired control/treatment arm, no repetitions,
no 30-run floor, so this is diagnostic data and explicitly **not** a promotion
signal. In the post-repair run the model never created the file at all, so the
new erasing-write warning did not fire live; it is unit-tested only. Two further
observations were recorded but not repaired: the runaway-loop guard did not stop
three identical `npm run test` verifications, and the recipe requests a `council`
extension the CLI rejects as unknown (`Failed to start extension 'council'`),
continuing without it.
