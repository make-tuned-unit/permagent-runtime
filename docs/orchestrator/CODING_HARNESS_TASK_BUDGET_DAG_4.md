# Coding Harness Task-Budget Boundary DAG 4

Status: active — 2026-09-04

## Objective

Make one user-approved coding task retain one authoritative spend boundary
across turns, compaction, retries, restarts, Council planning, and every child
worker. Continue using the existing Spectral session/cost ledger; do not create
a second budget store or infer spend from transcript text.

```text
B0 freeze task and ledger identities
 ├── B1 durable task identity + continuation ─┐
 ├── B2 exactly-once provider accounting ──┼── B4 enforcement seams
 └── B3 durable worker/goal attribution ───┘          │
                                                         ├── B5 projection + recovery
                                                         └── B6 adversarial integration
                                                              │
                                                              ▼
                                                    B7 promote routing DAG
```

## B0 — Contract freeze

- A new user-approved task may create a new budget identity.
- A turn, compacted continuation, retry, resumed session, Council pass, or
  worker replacement may not create a fresh budget for that task.
- Provider-reported usage and durable ledger rows are authoritative.
- Unknown cost remains unknown; local/subscription billing is never relabelled
  as paid API spend.

Gate: one typed task identity maps to one durable budget envelope.

## Gate ledger

| Node | State | Evidence required to pass |
|---|---|---|
| B0 | passed | durable task ID contract and single Spectral authority |
| B1 | passed | restart/history replacement, parent-child, and new-task fixtures |
| B2 | passed | concurrent/restart idempotency plus sparse provider-frame fixtures |
| B3 | passed | durable internal/external lineage; unknown token/spend fails closed after restart |
| B4 | passed | reservation/accounting gates, zero-unwrapped strict inventory, and integrated orchestrator fixtures pass |
| B5 | active (B5.0/B5.1 passed; B5.2/B5.3 implemented) | versioned BudgetProjection contract, canonical SessionManager/Spectral query seam, daemon/API projection, and CLI announcement compatibility are implemented; B5.4 Build and B5.5 integration remain |
| B6 | planned | adversarial integration matrix |
| B7 | planned | exact P4 receipts transition the program to P5 |

### B4 seam ledger

| Seam | State | Current evidence / remaining gate |
|---|---|---|
| reservation transaction + schema repair | passed | concurrency, restart, settlement, expiry, sibling scope, and existing-DB repair fixtures |
| conservative price/retry bound | passed for authorization safety | deterministic canonical/worst-case tests fail closed without an authoritative upper bound; reducing conservative over-reservation is an efficiency follow-up for P5 and cannot weaken this safety floor |
| primary Agent stream | passed | the turn path reserves the captured provider/model before `stream_split`, settles one authoritative terminal usage snapshot under the same invocation ID, and marks post-dispatch errors, missing usage, and cancellation unknown. Two primary-stream regression fixtures plus six existing fast/retry/unknown fixtures passed; the integrated library check passed. |
| external goal CLI | passed | typed configured billing; paid/unknown adapters refuse before worktree/process; focused denial/billing fixtures and integrated library check pass |
| compaction + `complete_fast` | passed (B4.3 only) | shared reserve/settle helper wraps each physical fast/full attempt and async tool-pair summaries; paid/unknown requires a durable task before dispatch, errors/missing usage/cancellation become unknown, and local fallback has distinct physical attribution. Six deterministic focused fixtures and the production dispatch inventory pass. B4 remains active: the other inventory seams are still unwrapped. |
| orchestrator summaries/decomposition | passed | conversation summaries and both roadmap-decomposition physical attempts now enter the shared `AccountedFastCompletion` seam; each attempt receives a fresh invocation ID and immutable provider/model snapshot. Six deterministic accounting/no-provider fixtures passed, and `cargo check -p permagent --lib` passed. B4.11 also repaired the verify-loop fixtures to create authoritative Spectral sessions/task identities and repaired the task-ceiling fixture to attribute its ledger row to the active task; the four regressions, full orchestrator module, and accounting gates passed without weakening fail-closed production behavior. |
| initiative draft + meeting write-up | passed | the initiative draft and session-provider meeting extraction now resolve durable hidden Spectral sessions and enter the shared `AccountedFastCompletion` seam; two deterministic source-contract guards passed, the integrated `cargo check -p permagent --lib` passed, and the production inventory contains no direct completion seam in either module. |
| playbook synthesis | passed | the background distiller now resolves a durable hidden Spectral session and enters the shared `AccountedFastCompletion` seam; its deterministic source-contract guard passed, the integrated `cargo check -p permagent --lib` passed, and the production inventory contains no direct completion seam in the module. |
| librarian atoms + summarize | passed | librarian atom generation and the summarize extension now enter the shared full-model `AccountedFastCompletion` seam while preserving the actor model and existing deterministic fallbacks; two source-contract guards and two no-paid-dispatch fixtures passed, the integrated `cargo check -p permagent --lib` passed, and neither module remains in the production completion inventory. |
| Council rounds/chair/re-ask | passed | reserves before dispatch, settles exact usage, and marks error/timeout/cancellation/missing usage unknown; paid refusal dispatches zero; local/subscription attribution and integrated library check pass |
| provider delegates + session naming | passed | Anthropic and sovereignty wrappers are typed caller-owned transports under the primary-stream reservation; session naming is deterministic local housekeeping for both text and empty/attachment-only turns, with no provider fallback. Eighteen focused provider-base tests passed and the production inventory reports no unwrapped seam in these modules. |
| supervised CLI + direct/background calls | passed (B4.9/B4.10) | recipe creation, MCP `create_message`, both Apps generation paths, Doctor provider checks, Financier close judgment, permission read-only judgment, and adversary inspection use the shared durable accounting seam; focused source guards, paid refusal/settlement reuse, and integrated library checks pass. Worker/provider transports are covered by the typed ownership proofs below. |
| static dispatch inventory | passed (B4.8A–B4.8D) | The strict `dispatch-inventory.v2` audit reports 26 production seams: 3 wrapped and 23 typed exclusions, with zero residual unwrapped seams. B4.8A corrects test/body parsing and proves deterministic/local subprocesses; B4.8B proves caller-owned primary, provider-base, and Council transports; B4.8C proves external-worker/provider-CLI ownership; B4.8D proves provider delegates and removes paid session-name housekeeping. |

“Implemented” is not a pass state. A seam moves to passed only after its
focused tests and the integrated library check both succeed.

## B1 — Durable task identity, continuation, and restart

Trace every path that creates or resumes a coding turn. Bind it to the original
task budget before dispatch. Prove compaction, restart, retry, and after-turn
continuation cannot reset accumulated spend or attempt caps.

Gate: deterministic state-transition fixtures preserve the same budget ID and
used amount through every continuation path.

## B2 — Exactly-once provider accounting

Normalize provider streams to one final cumulative usage observation per
logical invocation. Give every invocation a stable idempotency key, and commit
its ledger row, token deltas, dollar roll-up, and cache roll-up in one existing
Spectral transaction. A retry/failover is a new invocation; a duplicated or
reordered usage frame is not.

Gate: duplicate and concurrent delivery of one invocation changes every
roll-up exactly once; two different invocations with identical token counts
both count; an injected write failure leaves neither a ledger-only nor a
token-only partial commit.

## B3 — Durable worker and goal attribution

Trace internal subagents and external CLI workers from parent session through
cost-ledger insertion and roll-up. An external run ID must resolve to a durable
Spectral session rather than a synthetic process label. Carry the approved
goal/card ID into provider accounting; do not reconstruct it from transcript
text or a branch name.

Gate: internal and external workers both have durable parent and goal lineage;
goal token/spend queries include every attempt after restart; unknown parents
remain unknown rather than fabricated.

## B4 — Enforcement seams

Read task and recursive descendant spend immediately before every paid model
invocation and external-worker dispatch, not only when a model happens to call
a tool. A query failure is an explicit unknown state that refuses new paid
work. Council calls use the same ledger and budget seam. Concurrent siblings
must reserve authorization atomically so both cannot spend the same remaining
allowance.

Gate: toolless replies, external CLIs, Council members, retries, and concurrent
siblings cannot cross a gate without the existing Decision Inbox approval.

### B4 execution order

Each item gets a distinct invocation ID. Reserve before dispatch; settle exact
usage and ledger attribution atomically; release only when dispatch provably
did not occur. A post-dispatch failure or missing usage becomes `unknown` and
continues to consume the hold until reconciled.

1. Use the typed `Provider::cost_tier` and conservative canonical price/limit
   planner. Local/subscription calls remain metered but need no paid hold;
   ambiguous transports remain paid/unknown rather than being inferred free.
2. Wire the primary stream in `agents/reply_parts.rs`, including the provider's
   physical retry envelope, then wire automatic and recovery compaction in
   `agents/agent.rs`.
3. Wire both attempts in `Provider::complete_fast` independently and cover the
   asynchronous tool-pair summarizer in `context_mgmt`.
4. Wire orchestrator conversation summaries and the two roadmap-decomposition
   attempts. Reserve the provider actually selected after fallback resolution.
5. Change Council completion to retain usage/provenance; meter every parallel
   member, chair synthesis, and verdict re-ask under the initiating or scheduled
   durable task identity.
6. Carry explicit billing/model/bound data through `GoalTask`; reserve external
   CLI work before worktree/process launch, settle supported Claude/Codex usage,
   and refuse unknown paid adapters before launch.
7. Give supervised CLI runs durable sessions and the same accounting path.
8. Route direct/background completions (recipe/app generation, file summary,
   librarian atoms, MCP sampling, security/permission review, Financier,
   doctor, meeting write-up, initiative draft, playbook synthesis, and remote
   MESH fallback) through the shared seam.
9. Add a static bypass inventory test for production `complete`,
   `complete_fast`, `stream_split`, remote generation, and external process
   spawn sites. Embeddings require either the same contract or an explicit,
   tested exclusion from the coding-task budget.

Sub-gate: no code search result may identify a production paid-call site that
lacks a reservation/accounting wrapper or a typed non-paid/excluded proof.

## B5 — Projection and recovery

### B5.0/B5.1 child ledger

| Seam | State | Evidence |
|---|---|---|
| versioned `BudgetProjection` contract | passed | separate task/session cap triplets, settled/held/unknown/effective/remaining fields, explicit zero-vs-null semantics, band, billing evidence, task/root identity, completeness, provenance, and `asOf` |
| canonical Spectral query seam | passed | `SessionManager::budget_projection` recomputes from `sessions`, `cost_ledger`, `cost_reservations`, and current config in one SQLite read transaction; child/grandchild callers resolve the canonical root; SQL aggregates and bounded latest-evidence reads use indexed task/tree keys; same-root siblings are included once and unrelated roots excluded; no new store/table or copied spend snapshot |
| fail-closed arithmetic | passed | invalid/nonfinite data, invalid caps, unbound tasks, unknown holds, and query failures never fabricate zero or positive authorization; remaining is finite and clamped |
| B5.0/B5.1 verification | passed | thirteen projection fixtures including transaction/snapshot contract, grandchild/root equivalence, active-hold precedence, chronology/tie-break, zero-cost estimated parity, cap parity, ancestor-cycle rejection, golden serialization, unrelated-root exclusion, and bounded-query source/index proof; proportional `cargo check -p permagent --lib`; receipt `CODING_HARNESS_B5_0_1_PROJECTION_RECEIPT.md` |
| B5.3 CLI announcement | implemented; runtime promotion pending | Spend response/event retain legacy scalar fields, attach canonical `budget-projection.v1`, reject unavailable/bound-unknown projections with 503 before emitting, preserve explicit zero/unbound/hold semantics, and check CLI HTTP status; daemon/CLI library checks and deterministic helper/serialization coverage; receipt `CODING_HARNESS_B5_3_CLI_ANNOUNCEMENT_RECEIPT.md` |

Expose cap, used, remaining, billing class, and evidence provenance on the
existing run/goal projection. When the cap is reached, park before the next
paid dispatch and retain the exact minimum decision needed to continue.

Gate: UI/API/CLI projections agree with the ledger after restart and never show
negative remaining spend or false zeroes.

## B6 — Adversarial integration

Run no-model fixtures for retry storms, compacted continuations, daemon death,
duplicate child completion, mixed local/subscription/API workers, unknown
usage, and a cap crossed between worker selection and atomic dispatch claim.

Gate: no duplicate charge, reset, unauthorized paid dispatch, or unbounded
retry; existing approval, verification, and dispatch tests remain green.

## B7 — Successor

When all gates pass, transition P4 with exact receipts. The program controller
must activate P5 automatically; no human gate is required. Externally billed
provider comparisons remain deferred until a separate spend cap is approved.
