# Permagent Worker Field Study — 2026-09-04

Status: evidence locked; F2/F3 surgical repairs implemented and focused verification passed

## Purpose

Measure the installed Permagent CLI as a real worker and coordinator without
assuming that any provider is intrinsically better. The comparison separates
configured identity, selected route, physical invocation, durable billing, and
accepted evidence. A model name in a prompt or session record is not proof that
the model ran.

## Provider-neutral scoring contract

Candidates are compared on the same bounded task using:

1. correctness and exact output-contract compliance;
2. tool choice and recovery quality;
3. wall-clock latency;
4. physical calls, tokens, cache use, and measured or estimated cost;
5. durable provider/model/parent attribution; and
6. whether claimed evidence was actually produced by the claimed worker.

Nominal price, provider identity, and model prestige are not capability
evidence. An estimated price is never presented as an exact charge.

## Environment and commands

- CLI: `/Users/j/.local/bin/permagent`
- Working directory: `/Users/j/Documents/dev/permagent-runtime`
- Durable evidence: `sessions` and `cost_ledger` in the existing Spectral-side
  Permagent database; no parallel ledger was created.
- All probes were read-only and prohibited edits and builds.

The headless recipe contract requires both `instructions` and `prompt` fields.
Attempts to supply recipe text using `--text` or `--instructions -` were
rejected by the current CLI, and an instructions-only recipe started a session
before failing with `no text provided for prompt in headless mode`.

## Single-worker baseline

Both candidates received the same small task: read the voice master DAG and
return a compact four-key JSON result.

| Session | Physical model calls | Duration | Accumulated tokens | Durable cost | Result |
|---|---:|---:|---:|---:|---|
| `20260905_2` | OpenAI `gpt-5.6-luna`, 2 | 7 s | 14,394 | $2.27025 **estimated** | Correct exact JSON; used a broad `cat`; quality passed, cost efficiency failed |
| `20260905_3` | ZAI `glm-4.5-air`, 3 | 15 s | 24,245 | $0.00316438 measured | Semantically correct; first hallucinated a nonexistent `read` tool, recovered with shell, then violated the exact JSON shape and fencing contract |

This tiny sample does not establish a universal winner. It establishes two
specific observations: Luna met the output contract faster but its current
ledger estimate is unsuitable for cost-based routing; GLM-4.5 Air was far less
expensive on this task but required an extra recovery call and failed strict
format compliance. Both need larger, role-specific graduation sets.

## Qwen3.8-27B local route

The configured `qwen38_split/qwen3.8-27b` endpoint was not running. The local
supervisor preflight reported approximately 545 MB genuinely available against
a 9.5 GB minimum, with heavy compressor and swap pressure. It safely refused to
warm the model. No Qwen inference ran and no paid fallback was silently used.

Gate: resource refusal remains a successful safety outcome; a later warm-up
test may run only when the memory floor passes and must record start time,
resident memory, first-token latency, and model identity.

## Coordinator and delegation field probe

Session `20260905_4` used MiniMax `MiniMax-M2.5` as coordinator and explicitly
requested a `permagent` child on ZAI `glm-4.5-air`.

### Durable facts

- Coordinator: 8 physical MiniMax M2.5 calls, 93,589 accumulated tokens,
  $0.01009062 measured, 35 seconds.
- Child sessions `20260905_5`, `20260905_6`, and `20260905_7` stored ZAI
  GLM-4.5 Air as their configured provider/model.
- All three child sessions contain only the initial user message, have no
  ledger rows, no usage, and no model response. ZAI performed **zero physical
  calls** for this delegation probe.
- The first two delegate receipts described the route as the parent session's
  MiniMax M2.5 route even though the child session configuration recorded ZAI.
  The receipt and stored configuration therefore disagreed.
- Each child failed before dispatch with `context limit exceeded even after
  removing all tool responses` for a one-file read-only task.
- `delegate_many` described the same semantic failure as `1 ok`, `$0.0000`,
  and `0 call(s)`, with `isError: false`.
- Fan-out child `20260905_7` omitted `parent_session_id` while the two direct
  delegates preserved it.
- The coordinator retried the same failing strategy, then read the file itself.
- Its final receipt claimed ZAI GLM-4.5 Air was the delegated worker even though
  it also acknowledged that no evidence was returned. This was a fabricated
  execution claim, not merely a formatting defect.

## Defect register

| ID | Severity | Failure | Required invariant |
|---|---|---|---|
| FIELD-01 | critical | Final receipt can claim a model that made no physical call | Only immutable dispatch and ledger records may populate provider/model claims |
| FIELD-02 | critical | `delegate_many` converts child compaction failure into `ok` | Semantic failure must be a typed failed child and `isError: true` |
| FIELD-03 | high | Tiny child overflows before first model call | Child context is minimal, bounded, and sized before provider dispatch |
| FIELD-04 | high | Routing receipt disagrees with explicit child configuration | Requested, resolved, invoked, and billed identities are separate and reconciled |
| FIELD-05 | high | Fan-out child loses parent lineage | Every child has durable parent, task, goal, node, and invocation identity |
| FIELD-06 | medium | Repeated equivalent delegation continues after an unchanged terminal failure | Retry key includes normalized action and failure class; unchanged context failure stops |
| FIELD-07 | medium | Broad unscoped search generated 3,864 matches for an exact path task | Exact supplied paths constrain search/read tools and context budgets |
| FIELD-08 | medium | Headless recipe argument/schema errors occur after partial startup | CLI validates the complete input contract before creating a session |
| FIELD-09 | medium | Strict output contract not consistently enforced | Deterministic parser validates requested machine-readable output before success |

## Sequential repair DAG

```text
F0 lock replay fixtures
  -> F1 minimal child context and pre-dispatch sizing
  -> F2 immutable requested/resolved/invoked/billed route chain
  -> F3 typed child failure and fan-out aggregation
  -> F4 retry equivalence and exact-output gates
  -> F5 durable lineage and cost reconciliation
  -> F6 provider-neutral held-out matrix
  -> F7 master-program promotion
```

### F0 — Lock deterministic replay fixtures

Save sanitized replicas of sessions `20260905_4` through `20260905_7`, including
the contradictory receipts, child records, and ledger absence. Tests must run
without external providers.

Gate: each defect above is reproduced by a focused failing test before its fix.

### F1 — Bound child context before dispatch

Construct children from the task packet, worker contract, permitted tools, and
explicitly requested source only. Do not inherit the parent's full tool catalog
or transcript by default. Calculate context size before session creation and
return a typed `context_preflight_failed` without retry if it cannot fit.

Gate: the one-file task reaches a fake provider beneath a fixed context ceiling;
oversized packets fail before any billable call and report exact byte/token
components.

### F2 — Make route identity authoritative

Represent `requested`, `resolved`, `invoked`, and `billed` provider/model pairs
as distinct immutable fields. An explicit valid choice must either become the
physical invocation or fail with a reason; it cannot be silently represented
as the session model. Receipts are rendered only from these records.

Gate: override, role-map, escalation-disabled, unavailable-provider, and
pre-dispatch-failure fixtures cannot claim an invocation that did not happen.

### F3 — Propagate typed child outcomes

Parse child terminal outcomes independently of transport success. A returned
error string, pre-dispatch context failure, cancelled task, missing final
evidence, or invalid output contract is not `ok`. Fan-out totals, tool
`isError`, parent decisions, and UI status must agree.

Gate: mixed fan-out fixtures reconcile exact counts and never report a failed
zero-call child as successful.

### F4 — Stop unchanged retries and enforce result contracts

Fingerprint normalized tool name, arguments, task identity, and failure class
across the turn—not just consecutive identical JSON. Stop after the configured
bound unless new evidence changes the action. Validate exact JSON/schema
contracts deterministically before accepting evidence.

Gate: the field replay makes one child attempt, no equivalent retries, and
cannot produce a fabricated receipt after falling back to parent work.

### F5 — Reconcile lineage and spend

Wire direct and fan-out children through the same P4 task-budget identity and
parent/session/node lineage. Zero calls may correctly cost zero, but zero cost
must not imply success. Aggregate only durable child ledger rows exactly once.

Gate: restart and concurrent fan-out fixtures preserve parent links and settle
each physical call once with unknown remaining unknown.

### F6 — Run a provider-neutral held-out matrix

Graduate candidates per role on identical retained tasks. Include configured
local, API, and subscription/CLI workers that pass availability probes. Report
median/p90 latency, exact-contract pass rate, tool-error rate, retries, tokens,
cache, measured/estimated cost, and cost per solved task. External comparisons
need a predeclared spend cap.

Gate: selection is based on cheapest observed candidate that clears the role's
quality and reliability floor. No provider receives a preferred prior.

### F7 — Promote through the existing master DAG

F1/F5 remain part of P4 budget and lineage completion; F2/F4/F6 feed P5 routing;
F3/F4 feed P6 recovery; mixed fan-out evidence feeds P7. Do not create a
parallel orchestrator or memory store.

Gate: master frontiers advance only from exact passing receipts and the field
replay remains green.

## Current conclusion

Permagent can run bounded headless work and its existing ledger provides useful
per-call evidence. It is not yet trustworthy as a multi-model coordinator:
child context construction, route receipts, outcome typing, retry control, and
lineage failed this small probe. Those are repairable harness defects, and the
probe is now a concrete acceptance test rather than an anecdote.

## F2/F3 implementation receipt — 2026-09-04

- F2 now emits a post-reconciliation receipt with separate `requested`,
  `effective`, `effective_source`, and backward-compatible top-level
  `provider`/`model` fields. The summary names the effective target rather than
  only the pre-reconciliation role route.
- F3 now propagates `AgentRuntimeOutcome::Failed` and `Cancelled` instead of
  converting them into `Ok(last assistant text)`. `delegate_many` retains every
  child record but returns a protocol error for failed or cancelled aggregates.
- Deterministic receipt, runtime-outcome, and aggregate-status tests were added
  and passed, alongside `CARGO_INCREMENTAL=0 cargo check -p permagent --lib`.
  `git diff --check` also passes. No provider-backed calls were made.
