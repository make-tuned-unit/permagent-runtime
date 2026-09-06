# Coding Harness Capability and Cost Routing DAG 5

Status: staged — activates automatically after P4 receipts pass

Field evidence: `docs/orchestrator/PERMAGENT_WORKER_FIELD_STUDY_2026-09-04.md`
records a live provider-neutral CLI probe. Its FIELD-01 through FIELD-09
failures are mandatory R4/R5/R7 regression inputs, not optional observations.

## Objective

Choose the least expensive configured worker that has proved it can complete
each DAG role, while preserving explicit user model choices, durable context,
bounded fallback, and honest billing. Provider names, model prestige, prompt
text, and nominal token price are not capability evidence.

```text
R0 freeze routing contracts
 ├── R1 canonical configured inventory + billing classes
 └── R2 role-specific graduation evidence
          │
          ▼
 R3 deterministic cheapest-capable selector
          │
          ├── R4 explicit user choice + next-turn switching
          ├── R5 bounded fallback + context-preserving handoff
          └── R6 Council escalation policy
                    │
                    ▼
          R7 adversarial routing matrix
                    │
                    ▼
          R8 promote recovery DAG
```

## Gate ledger

| Node | State | Evidence required to pass |
|---|---|---|
| R0 | staged | typed capability, billing, override, and evidence contracts |
| R1 | planned | configured providers/CLIs/MCPs resolve without secret leakage or name inference |
| R2 | planned | every selectable role has fresh, reproducible graduation evidence |
| R3 | planned | cheapest-capable decisions are deterministic and cost-per-solved aware |
| R4 | planned | terminal and defaults honor provider/model choice on the next turn |
| R5 | planned | retry/escalation is bounded, attributable, and preserves the work packet |
| R6 | planned | Council is invoked only by typed ambiguity/risk thresholds and shares the task budget |
| R7 | planned | deterministic, local, and spend-capped held-out matrices pass |
| R8 | planned | exact P5 receipts activate P6 without a human continuity gate |

## R0 — Contract freeze

- One canonical capability card identifies provider, model, transport, auth
  readiness, context limit, tool reliability, supported role, latency evidence,
  billing class, price evidence, and evidence freshness.
- Billing is a closed typed value: `local_free`, `subscription`, `paid_api`, or
  `unknown`. Unknown never becomes free by provider-name convention.
- An explicit user selection outranks automatic routing until the user clears
  it or the selected worker is unavailable; the system explains the smallest
  required fallback rather than silently switching.
- Selection optimizes observed cost per solved node after capability gates,
  not nominal token price and not a global model ranking.

Gate: the schemas reject incomplete billing, stale evidence, ambiguous model
identity, and unsupported role claims.

## R1 — Configured inventory and probes

Unify the already-configured API providers, local runtimes, Cursor/Codex/Claude
CLIs, and MCP-backed capabilities into the existing provider inventory. Probe
availability and auth without executing paid work or exposing credentials.
Record transport-specific constraints and distinguish a subscription CLI from
a metered API explicitly.

Gate: restart-safe fixtures resolve each configured provider/model/CLI exactly
once; missing auth and probe failure remain unavailable/unknown, never free.

## R2 — Role-specific graduation

Graduate models independently for coordinator, planner, implementer, reviewer,
researcher, test author, and recovery roles. Use deterministic fixtures first,
then local retained smokes. Store the evaluator version, task-set hash, pass
rate, latency distribution, token use, measured/estimated cost, and expiry.
Project memories may brief a candidate but may not count as test evidence.

Gate: ungraduated and stale candidates cannot coordinate or verify; every
graduated role claim points to reproducible artifacts in the existing run
ledger.

## R3 — Cheapest-capable selection

Filter by required tools, context, write scope, privacy, billing permission,
and current graduation. Rank survivors by observed cost per solved node with
deterministic tie-breakers and a latency ceiling. Keep the selection function
pure; write its inputs, candidate exclusions, winner, and evidence IDs to the
existing routing snapshot.

Gate: a frozen matrix selects the same winner after restart, never selects an
incapable cheap model, and reconciles projected cost with settled task spend.

## R4 — Explicit choice and switching

Make provider/model selection use the same configured inventory in Terminal,
Chat, Voice, and their defaults. Selecting a provider reveals only its models;
the chosen pair takes effect on the next turn and is visible before dispatch.
Unavailable choices remain visible with a concrete reason and do not silently
fall back.

Gate: component and runtime fixtures prove the displayed provider/model,
persisted override, next-turn invocation, and ledger attribution agree.
The receipt must distinguish requested, resolved, physically invoked, and
billed identities; a configured child that fails before dispatch has no
invoked identity and may not be narrated as having run.

## R5 — Bounded fallback and handoff

Classify failures before fallback: auth/configuration, context, transient
transport, tool incompatibility, verifier failure, or capability failure. Use
bounded same-tier retries before a single graduated escalation when policy
allows it. Preserve the Spectral context packet, diff, failing command, and
verification receipt so a replacement continues rather than restarts.

Gate: retry storms terminate, escalation consumes the same task budget, every
switch has one reason, and no failure path loses or duplicates work.
Pre-dispatch context overflow is terminal until the work packet changes, and a
transport-successful tool result containing a typed child failure remains a
failed child.

## R6 — Council escalation

Invoke Council only for typed architectural ambiguity, conflicting high-risk
evidence, or an explicit user request. The orchestrator may suggest Council
non-blockingly; automatic invocation is allowed only under the approved task
policy and budget. Members are selected per role from graduated candidates,
all rounds are metered through P4, and the chair emits a surgical DAG rather
than source rewrites.

Gate: ordinary fixes stay single-worker; qualifying fixtures convene the
smallest useful Council; every member, synthesis, and re-ask is attributable.

## R7 — Adversarial matrix

Cover unavailable preferred models, stale capability evidence, forged cheap
billing, identical aliases from two providers, subscription/API ambiguity,
context overflow, CLI crash, rate limit, verifier disagreement, Council
parallelism, restart between selection and dispatch, and user override during
an active turn. Replay the `20260905_4` delegation incident: explicit child
override, zero-call compaction failure, fan-out aggregation, missing lineage,
equivalent retries, parent fallback, and attempted fabricated receipt.

Run deterministic and local tests first. Any externally billed comparison
requires the P8-style explicit spend cap and records the exact cap in the run.

Gate: the matrix shows correct routing, bounded fallback, complete attribution,
no unauthorized spend, and no silent user-choice override.

## R8 — Successor

On exact exit receipts, transition P5 and activate P6 recovery/fault injection
through the master program controller. This transition has no human approval
gate; only paid benchmark spend, policy threshold changes, or irreversible
release effects may interrupt continuity.
