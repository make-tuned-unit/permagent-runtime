# Permagent Coding Harness Excellence DAG

Companion-wide integration parent: [Permagent companion program](PERMAGENT_COMPANION_PROGRAM.md).
It maps the six product weaknesses onto this existing harness and the voice
program without replacing their scheduler, resetting progress, or waiving gates.

Status: active improvement program, started 2026-09-04. No area may be labelled **Excellent** until its graduation checks pass. The present local-model coordinator baseline is red (0/1 on the retained FizzBuzz smoke).

Machine-readable program controller:
`docs/orchestrator/CODING_HARNESS_MASTER_PROGRAM_DAG.yaml`. Its validator lives
in `permagent-eval::program` and enforces a continuous active/ready/blocked
frontier above these child DAGs. Runtime dispatch continues to use Permagent's
existing approved roadmap and goal-transition engine; this is not a second
scheduler or memory system.

## Outcome

Make Permagent a high-trust, provider-neutral coding orchestrator that always represents coding work as a bounded DAG, delegates each node to the least expensive capable worker, verifies proportionately, learns only from measured outcomes, and preserves Permagent's own product identity.

This program is not a feature-copying exercise. We use public behavior and design principles as research inputs, then implement Permagent-native mechanisms around Council, Spectral, the Decision Inbox, provider routing, durable goals, and the run ledger. Every borrowed principle needs a distinct Permagent mechanism and an acceptance test.

## DAG

```text
E0 Baseline + measurement contract
 ├── E1 Trust and DAG invariants ───────┐
 └── E2 Context and tool economy ── E3 Routing and cost ─┐
                                                        ├── E4 Recovery and long-run reliability ─┐
                         E1 + E3 ────────────────────────┤                                      │
                                                        └── E5 Multi-worker orchestration ───────┤
                                                                                                 ▼
                                                                                 E6 Continual improvement
                                                                                                 │
                                                                                                 ▼
                                                                                 E7 Held-out qualification
                                                                                                 │
                                                                                                 ▼
                                                                                 E8 Release or rollback
```

The bounded M0-M7 defect graph in
`docs/orchestrator/CODING_HARNESS_EVAL_LOOP.md` overlays every E-node. A
focused, integrated, or held-out regression reopens the earliest owning node;
passing a downstream check cannot conceal it. The machine-readable master adds
`p7b_continual_defect_loop` before held-out qualification to prove that this
feedback mechanism itself retains three clean frozen-slice iterations.

Each node has an owner, an artifact, a deterministic verifier, a rework budget, and an escalation condition. A node may not self-approve. A failed check with unchanged code and inputs is retained as evidence and is not rerun; the worker must change the implementation, change the diagnostic, or stop at the attempt cap.

## Excellence contract

Scores are calculated from retained run records. `Excellent` means all required metrics pass on three consecutive qualifying runs, with no unresolved P0/P1 defect and no missing evidence. A small sample may diagnose a weakness but cannot graduate an area.

| Area | Excellent graduation gate |
|---|---|
| Safety and trust | 100% of adversarial approval, filesystem, network, credential, and landing tests block unauthorized effects; zero verifier bypasses; terminal outcomes retain evidence. |
| DAG enforcement | 100% of coding requests publish a valid graph before model spend; every active node exists and has satisfied dependencies; coordinator mutations are zero outside explicitly authorized one-node work. |
| Verification integrity | 100% of code goals declare runnable acceptance evidence; zero vacuous passes; stale evidence cannot land; verification depth matches risk; identical failed gates are never repeated unchanged. |
| Observability | 100% of qualifying runs record task/version, route, provider/model, timings, tokens, billing class, tool calls, retries, gate attempts, evidence, result, and parent/child attribution. |
| Cost routing | At equal model, task set, limits, and pass rate, treatment improves USD/solved by at least 20% from its locked baseline or is within 10% of the cheapest passing reference; subscription/local usage is not misclassified as paid API spend. |
| Token efficiency | At non-inferior pass rate, median input tokens/task improve by at least 20% from baseline or are within 10% of the best paired reference; reserved context headroom is never breached. |
| Latency | Harness-only overhead is at most 250 ms p50 and 1 s p95; paired time-to-first-tool and wall time are within 10% of the best non-inferior reference; worker probes do not recur inside their valid cache window. |
| Cheap-model usability | A model is eligible for a role only after passing that role's smoke plus at least 80% of the curated micro-suite and 95% of tool/DAG-selection checks. Models that fail coordination remain eligible only for bounded mechanical nodes they can prove. |
| Coding success | At least 90% on the frozen internal suite, or within five percentage points of the strongest paired comparison harness using the same model and limits; no safety regression purchases the score. |
| Overall readiness | Every row above is Excellent for three qualifying runs; held-out evaluation passes; rollback is tested; the release report contains limitations and raw evidence links. |

Thresholds are versioned with the benchmark manifest. Moving a threshold after seeing a result requires a recorded Council decision and invalidates that result for promotion.

## Sequential work

### E0 — Lock the baseline and instrumentation contract

1. Freeze task IDs, fixture revisions, provider/model, reasoning setting, turn/tool/time/token limits, and random seeds.
2. Define a versioned run schema for parent and child events. Record `not_run` separately from `fail` when a budget stops execution.
3. Run a no-model conformance suite and one cheapest eligible model smoke. Preserve raw traces and environment metadata.
4. Generate an area scorecard from evidence; unknown data scores `Unrated`, never a pass.

Gate: two identical evaluator runs produce identical task selection and grading, and every required telemetry field is either populated or explicitly `unknown` with a reason.

### E1 — Trust and DAG invariants

1. Bind roadmap approval to the exact proposal hash and a single-use approval proof.
2. Enforce the coordinator/worker mutation boundary below the prompt layer.
3. Require current deterministic evidence before landing; code goals without a declared check are `uncertain`.
4. Add adversarial tests for forged approval, stale proof, direct dispatch bypass, cyclic/invalid graphs, synthetic continuation budget reset, and restart replay.
5. Persist partial evidence for failure, timeout, cancellation, and denial.

Gate: all trust tests pass under both direct API and effect-layer invocation, with mutation and decision ledgers reconciled.

### E2 — Context and tool economy

1. Measure fixed prompt, tool-schema, project-memory, retrieved-memory, and tool-output tokens independently.
2. Load the smallest role-specific tool surface. Fetch large instructions or MCP schemas only when the active node requires them.
3. Preserve decisions, modified paths, active DAG state, verifier commands/results, budgets, and unresolved failures through compaction; prune replaceable tool spew.
4. Keep Spectral as the memory system of record. Retrieval packets must be scoped, attributed, deduplicated, and evaluated for recall and contradiction; do not build a parallel memory store.
5. Localize likely files and relevant project memory before broad repository reads.

Gate: context-pressure fixtures retain every protected fact, memory retrieval meets the frozen precision/recall target, and paired token use satisfies the area threshold without a pass-rate regression.

### E3 — Routing and cost

1. Maintain a capability card per provider/model/CLI: availability, auth state, context limit, tool reliability, role graduation, latency, billing class, and observed cost.
2. Route by required capability first, then expected cost per solved node—not nominal token price.
3. Select provider and model atomically at dispatch. UI and terminal changes take effect on the next turn and are visible in the run projection.
4. Cache healthy worker probes with configuration-sensitive invalidation; reconcile child spend into the parent exactly once.
5. Escalate after evidence of incapability, not merely after a timeout; never spend an expensive Council on a one-node mechanical task unless risk or ambiguity justifies it.

Gate: a deterministic routing matrix chooses the cheapest graduated worker for every fixture, denies ungraduated coordinators, and reconciles estimated versus recorded spend.

### E4 — Recovery and long-running reliability

1. Give each external CLI dispatch an idempotency key and durable process identity.
2. On daemon restart, reattach or prove termination before retrying; never duplicate an unknown side effect.
3. Classify failures (model, provider, tool, verifier, environment, policy, budget) and choose a distinct recovery action.
4. Carry active goal, DAG, evidence, and budget across compaction and restart.
5. Stop after the node's rework budget and surface the best diagnostic plus the minimum user decision needed.

Gate: crash/restart, timeout, rate-limit, malformed edit, context overflow, and stuck-verifier fault injection all terminate correctly with no duplicate dispatch.

### E5 — Multi-worker orchestration

1. Council plans only when ambiguity, cross-system impact, or architectural risk crosses the configured threshold. It returns a typed DAG, not prose alone.
2. Decompose into nodes that a cheaper model can execute with explicit paths, constraints, inputs, acceptance criteria, and non-goals.
3. Parallelize only dependency-independent work and isolate overlapping write scopes. Merge through a designated integrator.
4. Give workers the minimum Spectral brief needed for codebase familiarity. Workers return evidence and a compact handoff, not raw context dumps.
5. Use an independent verifier for high-risk or cross-cutting changes. Review effort is proportional; low-risk deterministic success does not trigger a full Council.

Gate: synthetic and real multi-node tasks show correct dependency order, no write collision, complete child-cost attribution, and better or equal cost/latency than sequential execution at equal pass rate.

### E6 — Bounded continual-improvement loop

1. Run the M0-M7 graph for each diagnosed failure cluster; assign the defect to its earliest owning master/child DAG node.
2. Create candidate policy, prompt, skill, routing, or tool changes in an isolated branch/worktree from that one failure cluster.
3. Prefer the smallest evidence-backed change. Do not rewrite a subsystem to address a local failure.
4. Run the locked control and candidate on the same task IDs, model, limits, and environment. Keep held-out tasks inaccessible to the optimizer.
5. Compare correctness first, then trust, USD/solved, tokens, latency, tool and verification efficiency, and operational complexity. A candidate must not trade away a higher-priority invariant.
6. Reopen the owning node for any integrated or held-out regression. Send borderline, threshold-changing, or high-impact promotions to Council; otherwise use the deterministic promotion gate.
7. Promote the versioned candidate or roll it back. Store the hypothesis, diff, evidence, decision, and expiry/recheck condition through the existing Spectral/session path.

Loop stop conditions: all areas graduate; the iteration/token/dollar/time budget is exhausted; no candidate improves the Pareto frontier; or a P0/P1 requires human authority. No uncontrolled self-modification reaches the default harness.

### E7 — Held-out qualification

The deterministic scorecard contract is implemented in
`permagent-eval::qualification`; see
`docs/orchestrator/CODING_HARNESS_HELDOUT_QUALIFICATION.md`. It derives area
ratings from retained evidence and refuses to treat caller-supplied status
labels as proof.

Run gates in increasing cost order:

1. Per-change: local no-model conformance plus frozen internal micro-suite.
2. Nightly: a frozen BigCodeBench/LiveCodeBench component slice and a Terminal-Bench subset, paired control/treatment.
3. Release: repository-stratified SWE-bench Verified/Multilingual or the next harder qualified set when the current set saturates.

Public datasets are evaluation inputs, not product behavior specifications. Training/generated fixtures and held-out qualification tasks must be disjoint.

Gate: three consecutive qualifying reports satisfy every area threshold and include confidence intervals or paired task-level deltas; one lucky run cannot promote.

### E8 — Release or rollback

1. Produce the scorecard, raw artifact index, provenance ledger, known limitations, and migration/rollback procedure.
2. Rehearse rollback and confirm existing sessions remain readable.
3. Require human approval only for release, externally billed benchmark spend above the configured cap, policy-threshold changes, or irreversible/high-impact effects.
4. Deploy progressively and compare production telemetry to the qualification envelope.

Gate: release approval references the exact build and evidence hashes; any area leaving its envelope automatically pauses rollout and opens a requalification DAG.

## Worker lanes

| Lane | Default worker policy | Scope | Required handoff |
|---|---|---|---|
| Evaluation trainer | Cheapest model graduated for test/data work; stronger reviewer for metric changes | evaluator, paired loop, manifests, scorecard | commands, raw result paths, metric/schema diff, budget use |
| Trust verifier | High-reliability model; deterministic enforcement below it | approval proofs, landing, DAG validity, adversarial fixtures | invariant, exploit attempted, evidence, residual risk |
| Routing optimizer | Cheap analysis worker; capable integrator | capability cards, cost/latency routing, provider/model switching | paired route decisions, spend reconciliation, fallback proof |
| Context/memory specialist | Model graduated for retrieval analysis | Spectral packets, compaction, localization, tool-surface reduction | recall/precision, contradictions, token delta, protected facts |
| Reliability specialist | Model graduated for systems fault diagnosis | idempotency, restart/recovery, long commands | injected fault, state transition, duplicate-effect proof |
| Council/integrator | Strong model only when threshold trips | architecture, conflicts, promotions, high-risk review | typed DAG/decision, dissent, assumptions, exact acceptance gates |

The orchestrator may replace a worker only between node attempts and must record why. Provider choice is not a status symbol: the least expensive graduated worker wins.

## Source-to-Permagent provenance

| Research input | Principle retained | Permagent-native expression | Non-copying boundary |
|---|---|---|---|
| Codex | autonomous follow-through, narrow tool authority, subagent delegation, proportional verification | DAG envelope + sandbox policy + Decision Inbox + evidence ledger | retain Permagent Council/Spectral/routing and implement against our schemas and threat model |
| Claude Code | project context hierarchy, isolated subagents/worktrees, lifecycle hooks, inspectable configuration | Spectral briefs + scoped workers + typed lifecycle events + Command Center diagnostics | do not reproduce Claude commands/UI or prompt text; use our providers, event model, and interface |
| Prime Agent | persistent goals, bounded autonomous runs, compact state, small evidence-backed refinement | durable Permagent goal/DAG state + bounded candidate loop + Spectral promotion record | no self-editing default harness and no opaque REPL state as the source of truth |
| Pi | small provider-neutral core, explicit model registry, composable extensions, regression fixtures | thin orchestration kernel + capability cards + MCP/CLI adapters + deterministic tests | keep Permagent's integrated trust layer rather than externalizing safety |
| Permagent | Council, Spectral, cross-provider routing, run projection, approval/verification gates | the product architecture and source of truth | new work must strengthen these systems instead of creating parallel substitutes |

Primary references:

- Codex model and agent guidance: <https://developers.openai.com/api/docs/guides/latest-model>
- Claude Code subagents and project memory: <https://code.claude.com/docs/en/sub-agents>
- Claude Code lifecycle hooks: <https://code.claude.com/docs/en/hooks>
- Claude Code parallel-agent approaches: <https://code.claude.com/docs/en/agents>
- Prime Agent repository: <https://github.com/PrimeIntellect-ai/prime-agent>
- Prime Agent long-running agents: <https://docs.primeintellect.ai/prime-agent/long-running-agents>
- Prime Agent refinement: <https://docs.primeintellect.ai/prime-agent/skills/refine>
- Pi monorepo: <https://github.com/badlogic/pi-mono>

## Current evidence and immediate queue

- Existing deterministic evaluator: 129 tests; 17 task manifests validated at the last baseline.
- Existing harness gates cover the DAG envelope, coordinator mutation boundary, structured review, durable landing evidence, worker-probe caching, child spend rollup, and run-projection fields.
- Retained local Qwen coordinator smoke: 0/1, 102 seconds, 31,921 aggregate input tokens, one denied direct write, no required roadmap invocation. This model is **not graduated for coordination**.
- The broad repository suite is not a clean baseline: previously observed failures include sandbox-denied listener tests, host-path assumptions, and prompt snapshot drift. These must be classified, not hidden or counted as product failures without diagnosis.

Immediate order:

1. Finish active P4/B4 accounting enforcement: correct the production dispatch
   inventory, close the primary stream gate, wrap genuine background model and
   external-worker seams, and prove caller-owned/local exclusions without
   double accounting.
2. Complete P4 projection/recovery and adversarial integration, then transition
   to P5 only with the exact exit-gate receipts.
3. Add role graduation and deterministic cheapest-capable routing before
   rerunning any local coordinator smoke.
4. Continue through recovery and multi-worker nodes using the M0-M7 defect
   graph on every failed focused, integrated, or held-out check.
5. Run the cheapest deterministic gates first. Only after local gates pass,
   authorize a bounded externally billed paired run; do not jump directly to a
   broad benchmark sweep.
