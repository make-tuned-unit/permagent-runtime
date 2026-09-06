# Coding Harness Performance and Cost DAG

Status: implementation and local deterministic gates passed on 2026-09-04. External model benchmarks remain intentionally budget-gated.

## Objective

Every coding request has a machine-readable DAG before model spend. The harness routes each node to the least expensive healthy worker likely to satisfy its acceptance criteria, records its route and verification, and cannot land code without durable passing evidence. “Always a DAG” does not mean “always a large DAG”: a tiny request may have one execution node plus bounded verification.

## Execution policy

1. **Scope and route** — identify the smallest independently verifiable change, project constraints, risk tier, candidate worker, and cost ceiling.
2. **Execute** — dispatch the node to a worker. The top-level coding session coordinates and may not directly mutate files.
3. **Verify** — run the cheapest deterministic check that proves the stated acceptance criteria. Escalate to broader suites or a reviewer only when risk, shared code, or failed evidence requires it.
4. **Review and land** — an approval is actionable only after a durable verifier `pass`. Missing, uncertain, or failing evidence stays in Review.

The run projection schema exposes DAG nodes, edges, active node, selected worker/provider/model/tier, routing reason, verification command/verdict/attempts, pending gate, elapsed time, tokens, spend, retry/tool/gate counters, bounded evidence/result summaries, task version, and parent attribution. Invalid graphs—empty, cyclic, duplicate, self-referential, or with impossible active-node state—are rejected at ingestion. Schema support is not treated as runtime evidence: the current CLI heartbeat still leaves several of these fields unknown or at their compatibility defaults.

## Sequential gates

| Gate | Contract | Verification | Status |
|---|---|---|---|
| G0 Baseline | Existing evaluator is deterministic and tamper-resistant | `cargo test -p permagent-eval`; `permagent-eval validate` | Pass: 133 library + 12 CLI tests; 17 tasks valid |
| G1 DAG envelope | Every coding run publishes a bounded valid DAG before spend | CLI announcement and terminal-supervision tests | Pass |
| G2 Mutation boundary | Top-level coding coordinator cannot edit directly; dispatched child workers can | tool-inspection tests | Pass |
| G3 Review integrity | Reviewer request alone is insufficient; only a successful structured `APPROVE` counts | after-turn tests | Pass |
| G4 Landing integrity | Approval cannot land before durable verifier pass; API and effect layer both enforce it | decision-effect tests | Pass |
| G5 Cost/latency | Worker availability probes reuse a 5-minute config-sensitive cache; parent session budgets include direct child spend | probe and budget tests | Pass |
| G6 Contract visibility | Command Center API type includes the complete run/DAG/verification projection | TypeScript typecheck | Pass |
| G7 External signal | Hold the model fixed and compare harness variants on public tasks under an explicit dollar cap | staged Hugging Face plan below | Ready, not spent |

## Verification economics

Verification is proportional, with a hard upper bound on repeated attempts:

- **Low risk / localized:** targeted unit test, formatter, or static check; no Council.
- **Medium risk / shared seam:** targeted test plus adjacent contract test; one reviewer response.
- **High risk / security, data, billing, deployment, cross-system:** deterministic suite, independent reviewer, and explicit human approval.
- A repeated unchanged failure is evidence of a blocker, not permission for another identical loop. Park with the failing command and last evidence after the configured attempt cap.

## Hugging Face benchmark selection

Use only official publisher datasets and freeze both dataset revision and task IDs in every report.

### Tier A — per-PR, cheap signal

- Keep the 17 local Permagent tasks as the fastest harness wiring/cost regression gate.
- Add a frozen 20–30 task slice of **BigCodeBench Instruct** for Python tool/library correctness. Its official card has 1,140 tasks and high test coverage, but it primarily measures code generation rather than repository-level agent behavior, so it is a component diagnostic rather than the headline harness score: <https://huggingface.co/datasets/bigcode/bigcodebench>.
- Add a small, date-stratified **LiveCodeBench code-generation-lite** slice for fresh problem solving. Do not fetch the 9.38 GB full artifact in ordinary PR jobs: <https://huggingface.co/livecodebench>.

### Tier B — nightly harness signal

- Run a frozen 10–20 task subset of **Terminal-Bench**, which directly exercises shell use, environment recovery, long-running commands, and verification. Pin a release tag, not `main`; the official Hub mirror publishes immutable tags and currently documents 66 tasks in v4.0.0: <https://huggingface.co/datasets/harborframework/terminal-bench>.
- Hold provider/model and maximum turns constant across control and treatment. Compare paired task outcomes.

### Tier C — release qualification

- Run a repository-stratified subset of **SWE-bench Verified**. It contains 500 expert-verified real GitHub issues with repository/base commit, tests, and difficulty fields: <https://huggingface.co/datasets/princeton-nlp/SWE-bench_Verified>.
- Add **SWE-bench Multilingual** when the harness must prove routing and tool use across languages: <https://huggingface.co/datasets/SWE-bench/SWE-bench_Multilingual>.
- Reserve the full sets for periodic qualification because container setup and model inference dominate cost.

### Excluded from evaluation

**SWE-smith is a training corpus, not a clean held-out scorecard.** Its card describes tens of thousands of generated software-engineering task instances and recommends language-specific successor datasets. It can help generate adversarial fixtures, but must not be mixed into the held-out benchmark score: <https://huggingface.co/datasets/SWE-bench/SWE-smith>.

## Metrics and stopping rules

Record per task and aggregate:

- deterministic pass/fail/not-run and paired control/treatment delta;
- wall-clock time, time to first tool, tool calls, retries, verification attempts, rate limits, and cache-hit ratio;
- input/output/cache tokens, exact or estimated USD, median USD/task, and USD/solved;
- DAG validity, node count, routing choice/reason, gate count, reviewer verdict, and whether landing evidence was current;
- mutation surface (files/lines) and whether the worker stayed inside declared paths.

Stop a run when its shared dollar cap trips; mark remaining work `not_run`, never `fail`. Promote a harness change only when it does not regress pass rate, lowers either median latency or USD/solved, and introduces no trust-gate failure. Small samples are diagnostic only; do not promote on a single task.

## Local smoke result (2026-09-04)

The first retained `fizzbuzz` smoke found an evaluator wiring bug before model execution: a relative `--permagent-bin ./target/debug/permagent` was resolved after the runner changed into its scratch workspace. The runner now resolves path-like binaries against the operator cwd before spawning, and the documented relative-path form is covered by a unit test.

After that repair, `ollama/qwen25-16k:latest` completed the task process but did not solve it:

| result | wall time | input tokens | output tokens | tool requests | cost |
|---|---:|---:|---:|---:|---:|
| 0/1 | 102.0s | 31,921 | 172 | 1 (`write`, denied) | $0.0000 |

The runtime correctly denied the top-level direct write, but this 7B local model did not invoke the required roadmap tools and instead stopped. This is useful routing evidence: it is currently eligible for bounded mechanical child nodes, not for the coding-DAG coordinator role. Do not spend a 17-task sweep to repeat a proven startup behavior; re-run this smoke first after planner/tool-selection changes, and graduate the model only after it produces a real roadmap and passes the oracle.

## Remaining hardening backlog

These are separate, bounded DAGs rather than hidden scope in this pass:

1. Make approval-proof consumption and roadmap/card creation one durable transaction. Exact-proposal, one-use approval and direct-bypass denial are implemented, but a downstream creation failure can currently consume the proof and require re-approval.
2. Persist a logical task/run boundary so synthetic continuation messages cannot reset task spend accounting.
3. Propagate billing class into child-worker metering and durable cost records. The live CLI run projection now labels only confidently known local and subscription-CLI providers and leaves every other provider unknown; it does not guess that unknown means metered API.
4. **Completed in instrumentation DAG 2:** persist the live harness projection in the existing Spectral/session SQLite store. Prompt context remains live-only and is redacted before durable storage; session binding and terminal outcomes are monotonic across restart.
5. **Partially completed in instrumentation DAG 2:** retry/tool/gate counters and success/failure/denial/cancellation now come from structured runtime events. Parent attribution, verifier evidence, and timeout classification remain null until their authoritative producers are joined.
6. Refresh budget SQL once per turn (with reconciliation) instead of repeating three reads for each tool-inspection batch.
7. **Completed in instrumentation DAG 2:** project-memory and Spectral-recall blocks carry typed metadata to the exact request seam. The bridge filters non-memory sources before top-k, deduplicates stable keys, preserves anonymous hits distinctly, and accounts the exact installed/truncated text without logging memory bodies.

## Instrumentation audit — 2026-09-04

The locally reproducible instrumentation gates are green:

- `cargo test -p permagent-eval`: 133 library and 12 CLI tests passed.
- `cargo test -p permagent --lib context_packet::tests`: 7 passed.
- `cargo test -p permagent --lib harness_`: 15 passed, including durable restart storage and the late-heartbeat terminal race.
- `cargo test -p permagent-cli spend_announce::tests`: 14 passed, including event deduplication, poison recovery, and result precedence.
- `cargo test -p permagent-cli brain_sync::tests`: 12 passed, including exact recall attribution and stale-block clearing.
- `cargo check -p permagent-daemon -p permagent-cli -p permagent-eval`: passed.
- Command Center `npm run typecheck`: passed.
- Command Center Council escalation component tests: 2 passed.
- `git diff --check`: passed.

This establishes deterministic plumbing, not an Excellent rating. No area graduates until the retained held-out pipeline contains three consecutive complete qualifying runs. External provider/model sweeps remain spend-gated.
