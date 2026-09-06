# Permagent companion improvement sub-DAGs

Status: planned companion program; documentation-only artifact created 2026-09-05.
No production code, scheduler, controller, ledger, UI, or memory store is changed
by this file. A node is not a runtime pass: it must attach fresh evidence from the
commands/tests named below.

This document turns six observed weaknesses into one sequential chain while
preserving the existing Permagent controller, scheduler, Decision Inbox, and
Spectral session/cost/recognition substrate. It does not create a second DAG
engine, retry framework, budget store, status projection, or memory system.

Inherited boundary: P4/B5 (`CODING_HARNESS_TASK_BUDGET_DAG_4.md`) remains active
with B5.2/B5.3 implemented but B5.4/B5.5 still pending; B6 remains planned.
Those nodes must not be reset, renamed, duplicated, or declared passed by this
companion. Companion gates consume their exact receipts when available.

```text
C0 evidence lock / inherited-state check
  ↓
C1 boundary + fault reliability
  ↓
C2 backend → macOS/iOS/watchOS authoritative state
  ↓
C3 precise small-worker assignment + Spectral RecognitionContext
  ↓
C4 local-first measured-cost / verified-success routing
  ↓
C5 bounded risk-proportional verification
  ↓
C6 executable-plan / status / document consistency
```

## Common execution contract

- Every section has a read-only inventory node before an implementation node.
  Inventory records confirmed versus unconfirmed paths and the current source
  revision. No stale receipt, screenshot, or status label is accepted as proof.
- The existing approved-roadmap/goal-transition engine is the only coordinator.
  Existing Scheduler/SchedulerTrait remains the only scheduled-work controller.
- Spectral is the sole memory substrate. Recognition, operator corrections,
  provenance, and recall use existing Spectral/session paths; no vector cache,
  “reputation” database, sidecar memory, or duplicate ledger may be introduced.
- Trust remains private by default; no public reputation or blockchain identity
  work is authorized here, and MESH remains deferred. Existing identity and
  permission contracts are reused rather than replaced.
- Autonomy is earned only by task-scoped, measured, trustworthy repetition.
  Reputation never grants permissions, expands write scope, bypasses approval,
  or changes billing class. An operator correction is useful only when recalled
  with source/task/provenance and remains auditable.
- Full app/computer tools remain available only inside the existing permission
  boundary. A worker may not infer extra authority from platform, model, prior
  success, or a UI status.
- The charter allows at most two unsuccessful repair attempts per bounded node;
  this companion normally uses one changed-diagnostic or changed-implementation
  retry, then one independent verifier attempt. Identical inputs and unchanged
  code are never rerun as evidence. A further repair requires root diagnosis
  and a revised bounded node; Decision Inbox is needed only when authority,
  scope or spend changes, not for every ordinary repair.

## C0 — Evidence lock and inherited-state check

**Dependencies:** none. **Owner:** independent evidence worker; cheapest
read-only model. **Write scope:** this document's companion receipt only.

**Nodes**

| ID | Depends on | Work and exact target | Exit evidence |
|---|---|---|---|
| C0.1 | — | Record `git rev-parse HEAD`, dirty paths, active goal/card, and the exact B5/B6 receipts/status. Inspect `docs/orchestrator/CODING_HARNESS_MASTER_PROGRAM_DAG.yaml`, `CODING_HARNESS_EXCELLENCE_DAG.md`, and `CODING_HARNESS_TASK_BUDGET_DAG_4.md`. | `C0-baseline.json` with source hash, inherited states, and no claim that B5/B6 passed. |
| C0.2 | C0.1 | Inventory targets named in C1–C6 with `rg`, classify each as confirmed, unconfirmed, or out of scope, and record current focused-test commands. | `C0-target-inventory.md`; missing paths become blocked handoffs, never guessed paths. |
| C0.3 | C0.2 | Verify planning changes contain no production edits and implementation nodes reuse the existing scheduler/controller/memory/budget framework. | Read-only diff check plus controller-boundary statement. |

**Gate:** C0 passes only if inherited B5/B6 remain intact and every later node
has a source path, an owner, an acceptance command, and a rollback/evidence
location. Runtime health is explicitly `unknown` until its focused gate runs.

<a id="c1-boundary-reliability"></a>

## C1 — Boundary reliability

**Purpose:** close the gap between permission/budget/worker boundaries and
restart, timeout, malformed-input, duplicate-effect, or stuck-verifier faults.

**Dependencies:** C0.1–C0.3; C1 integration consumes B5/B6 receipts but does
not reproduce their tests. **Owner/model policy:** high-reliability trust or
systems worker for policy; cheapest capable deterministic worker for fixtures;
independent verifier for any boundary change.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed: `crates/goose/src/agents/platform_extensions/dispatch_scope.rs`,
  `write_scope.rs`, `goal_engine.rs`, `terminal_supervision.rs`,
  `supervised_cli.rs`, `execution_receipt.rs`, and `publish_sequence.rs`.
- Confirmed: `crates/goose/src/session/budget_projection.rs`,
  `cost_router/reservation.rs`, `cost_router/hold_done.rs`, and
  `crates/goose-server/tests/trust_boundary.rs`, `stream_lifecycle.rs`,
  `liveness_wire.rs`, `decisions_lifecycle.rs`.
- Confirmed: `crates/goose-server/src/routes/runs.rs` is best-effort and
  currently lacks live snapshots for several interval workers; whether this is
  in this section's implementation scope is **unconfirmed** and must be decided
  from the C1 inventory.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and integration gate |
|---|---|---|---|
| C1.1 | C0 | Build a boundary/fault matrix: caller, effect, approval, write scope, budget state, process identity, restart state, terminal outcome. Include direct API and effect-layer invocation. | Matrix has no “trusted because prior success” row; existing trust/stream/liveness tests are mapped, not assumed passed. |
| C1.2 | C1.1 | Define typed failure transitions for deny, unknown usage, timeout, cancellation, crash/restart, malformed edit, duplicate completion, and stuck verifier at the existing seams. | Pure transition fixtures prove one terminal outcome, no duplicate side effect, no budget reset, and no retry after an unknown side effect. |
| C1.3 | C1.2 | Add only the smallest production seam fixes required by failing C1.2 evidence; preserve existing Decision Inbox and B5 accounting ownership. | Use targeted `cargo test -p permagent --lib <filter>` plus affected `cargo test -p permagent-daemon --test trust_boundary --test stream_lifecycle --test liveness_wire` targets after inventory; failures are retained with class (product/infrastructure/environment). |
| C1.4 | C1.3 | Run an integration journey: approved coding task → worker dispatch → denial/timeout/restart injection → receipt/rollback → status projection. | Exact task/budget/session IDs reconcile; no paid dispatch crosses an unknown hold; a restart cannot replay an uncertain effect. B5/B6 receipts are referenced, not recreated. |
| C1.5 | C1.4 | Independent verifier reviews the diff against the matrix and checks rollback. | `C1-reliability-receipt.json` contains failed injections, terminal states, diff hash, commands, raw evidence links, and residual risk. |

**Consequence/security/rollback:** A false allow can expose files, spend money,
or duplicate an external action; a false deny parks work and surfaces the exact
operator decision. Roll back the node's isolated commit/worktree only after
preserving the failure receipt; never reset the shared worktree destructively.

**Bounded retry:** one fix attempt per failure class; one verifier; escalate on
any repeated unknown-side-effect, unauthorized-effect, or duplicate-effect
finding.

<a id="c2-authoritative-product-state"></a>

## C2 — Authoritative product state

**Purpose:** ensure one backend/session state is projected consistently through
the desktop/macOS Command Center, iOS voice/chat UI, and watchOS relay, including
terminal/recovery state and next-turn model semantics.

**Dependencies:** C1.4/C1.5; C2 cannot claim device success without a fresh
build. **Owner/model policy:** Swift-capable surgical worker for UI contracts;
Rust/API worker for wire contracts; independent device/evidence integrator.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed backend/wire: `crates/goose-server/src/routes/voice.rs`,
  `routes/config_management.rs`, `routes/runs.rs`, `routes/status.rs`,
  `routes/events.rs`, and `crates/goose-server/tests/events_wire.rs`.
  Whether each status route is the canonical source for the target journey is
  **unconfirmed**; the inventory must follow the actual route registration.
- Confirmed iOS: `ios/PermagentMobile/PermagentMobile/VoiceView.swift`,
  `VoiceProtocolTypes.swift`, `APIClient.swift`, `ModelPickerView.swift`,
  `Shared/WatchBridge.swift`, `HubWatchRelay.swift`, and their existing
  `VoiceIdleTests.swift`, `ChatStreamTests.swift`, `WatchBridgeTests.swift`.
- Confirmed watchOS: `PermagentWatch/WatchRelay.swift`, `WatchRecorder.swift`,
  `WatchHomeView.swift`. `Shared/WatchBridge.swift` explicitly documents that
  watchOS cannot run Tailscale and the iPhone is the hop; do not route around it.
  `WatchRelay` already persists queued recordings, tracks `activeTransferIds`,
  and has a response watchdog; C2 must add correlation/reconnect evidence around
  those seams rather than replacing the relay.
- Confirmed macOS/web surface candidates: `ui/command-center/src/ChatApp.tsx`,
  `lib/voiceHandoff.ts`, `components/voice/*`, and `lib/costMeter.ts`.
  Exact macOS packaging/runtime ownership is **unconfirmed** and must not be
  inferred from the iOS project name.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and journey gate |
|---|---|---|---|
| C2.1 | C1 | Trace one session/turn ID from backend creation and Spectral/session persistence through SSE/WebSocket/HTTP to desktop, iOS, and watch. Record every translation and terminal state. | A contract table identifies authoritative field, source, version, redaction, and stale-state behavior; no UI-local “truth” is promoted. |
| C2.2 | C2.1 | Lock state-machine fixtures for connecting/ready/listening/thinking/speaking/failed/empty/disconnected, model switch next-turn semantics, and watch queued/offline/reconnect behavior. Correlate `queuedRecordings`, `activeTransferIds`, watchdog completion, and request IDs. | Existing voice/watch/chat tests plus pure malformed/out-of-order/reconnect fixtures pass; unknown or stale state is visible as unavailable, never success. |
| C2.3 | C2.2 | Implement only surgical adapters/decoders/projection fixes at the confirmed owner files; keep WatchConnectivity as the watch hop and Spectral/session as authority. | Backend contract tests and iOS/watch/web focused suites pass; no alternate persistence or background sync service appears in the diff. |
| C2.4 | C2.3 | Fresh integration journey: macOS starts a turn, iOS captures/replies, watch queues then relays a note/chat, daemon restart occurs, and all surfaces converge on the same terminal/session state. | Fresh matching source/build IDs, logs, screenshots or simulator evidence, and event trace prove convergence. Device-only assertions remain `deferred` if hardware is unavailable. |
| C2.5 | C2.4 | Accessibility/privacy/security verifier checks screen-reader labels, no home/path disclosure, stale-build rejection, auth scope, and no transcript/audio leakage in telemetry. | `C2-state-journey-receipt.json` includes per-platform evidence and explicit unavailable hardware results. |

**Consequence/security/rollback:** Divergent state can cause duplicate sends,
lost notes, misleading “speaking/working” claims, or transcript disclosure.
Rollback is to the last wire-compatible client/server pair; preserve the
contract version and mark the affected surface unavailable rather than silently
falling back to guessed state.

**Bounded retry:** one protocol correction and one platform-specific correction;
one fresh-build rerun. Repeated device-only failure becomes `blocked` with the
exact hardware/build prerequisite.

<a id="c3-worker-and-memory-contract"></a>

## C3 — Worker and memory contract

**Purpose:** assign small workers only to tasks they can prove, provide compact
task-scoped context, and make recognition/learning provenance useful without
creating parallel memory or reputation permissions.

**Dependencies:** C2.5. **Owner/model policy:** cheapest graduated worker for
mechanical/read/retrieval work; stronger integrator only for cross-file or
ambiguous boundaries; RecognitionContext changes require an independent
Spectral-aware verifier.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed routing: `crates/goose/src/cost_router/role_map.rs`,
  `recommend.rs`, `derived.rs`, `worker_probe.rs`, `cheap.rs`,
  `agents/platform_extensions/summon.rs`, `goal_engine.rs`,
  `dispatch_brief.rs`, and `role_brief.rs`.
- Confirmed recognition: `crates/goose/src/recognition.rs`,
  `recognition_sink.rs`, `recognition_consent.rs`,
  `session/spectral_schema.rs`, and `docs/architecture/SPECTRAL_INTEGRATION.md`.
  `Brain::recognize()` query mode is wired; `StreamTracker` ambient mode is
  explicitly not wired because its lifetime/segment/chime-in contract is
  unresolved. Do not claim ambient recognition or invent that contract here.
- Historical caution: `docs/MEMORY_ARCHITECTURE_DECISION.md` is a May 3
  historical note, not live architecture evidence. C3 must verify current
  Spectral/session paths before relying on any old dual-memory counts or claims.
- **Unconfirmed:** exact operator-correction call sites beyond existing
  pronunciation/decision tools. Inventory `agents/platform_extensions/pronunciation.rs`,
  `librarian.rs`, `retrospect.rs`, and Decision Inbox persistence before any
  change; provenance must remain visible in existing Spectral/session records.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and journey gate |
|---|---|---|---|
| C3.1 | C2 | Produce a task card taxonomy: mechanical read/search, bounded edit, orchestration, review, local-only, and high-risk cross-system. Each card states paths, non-goals, inputs, acceptance command, write scope, budget, and escalation. | Every card maps to one `WorkflowRole`; half-configured/ungraduated roles fail closed; no role is inferred from reputation. |
| C3.2 | C3.1 | Define the compact Spectral brief: task/card/session IDs, relevant recalled facts with source/provenance, active DAG/budget/evidence, unresolved failures, and operator corrections; prune replaceable tool spew. | Retrieval precision/recall and contradiction fixtures pass; `RecognitionContext.session_id` is preserved; unresolved/orphan recognized IDs degrade safely as existing sink rules require. |
| C3.3 | C3.2 | Add or repair assignment/brief plumbing only at existing role-map/dispatch/recognition boundaries. Operator corrections teach/recall with provenance; they do not alter permissions, write scope, or billing class. | Focused role-map, worker-probe, recognition, and provenance tests pass; no second memory table/store/framework is introduced. |
| C3.4 | C3.3 | Journey: small mechanical worker handles a bounded file-read/test task, returns evidence, is denied an out-of-scope edit, then a capable integrator receives only the compact handoff and Spectral context. | Correct worker/model selected; denied effect recorded; task completion and recognition write-back join by durable IDs, not transcript text. |
| C3.5 | C3.4 | Independent verifier checks context injection for prompt-injection/data-origin boundaries and checks that repeated success never widens authority. | `C3-worker-context-receipt.json` has role decision, probe cache evidence, context hash, recognition/provenance IDs, and residual uncertainty. |

**Consequence/security/rollback:** Poor assignment wastes spend or lets a small
worker make unsafe edits; over-trusting recognition can turn familiarity into
unauthorized action. Roll back routing/brief changes to the prior role map and
disable the affected worker role; preserve Spectral records and do not delete
recognition evidence to hide a bad assignment.

**Bounded retry:** one role-map fix and one context/provenance fix; no repeated
retrieval benchmark with unchanged corpus/query. Ambient recognition remains
parked until its separately specified contract exists.

<a id="c4-local-first-routing"></a>

## C4 — Local-first routing

**Purpose:** choose the least expensive capable route using measured success and
real billing evidence, not nominal token price or fabricated local savings.

**Dependencies:** C3.5 and the inherited B5/B6 receipts when they pass.
**Owner/model policy:** cheap deterministic routing analyst for matrix work;
  systems integrator for reservation/ledger changes; paid provider calls only
  under existing approval and spend cap.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed: `crates/goose/src/cost_router/tier.rs`, `cheap.rs`, `budget.rs`,
  `reservation.rs`, `hold_done.rs`, `fallback.rs`, `escalation.rs`, `mesh.rs`,
  `canonical/cost.rs`, and `config/worker_probe.rs`.
- Confirmed: `crates/permagent-eval/src/cost.rs`,
  `crates/goose-server/tests/coding_spend_wiring.rs`, and
  `ui/command-center/src/lib/costMeter.ts`; the latter distinguishes coding
  harness spend, estimates, tokens, and child roll-up at the renderer boundary.
- **Unconfirmed:** the exact local model inventory, measured success baseline,
  and provider billing evidence available on the target machine. No local route
  may be labelled free or successful until observed in the receipt.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and journey gate |
|---|---|---|---|
| C4.1 | C3 | Freeze paired routing fixtures with same task IDs, limits, context, provider/model availability, and environment. Measure pass rate, verified-success rate, wall/TTFT, tokens, retries, and authoritative USD or `unknown`. | Local/subscription/paid classes remain distinct; unknown price/usage fails closed for authorization and is not rendered as a bill. |
| C4.2 | C4.1 | Build a deterministic capability matrix: required capability → graduated candidates → expected cost per verified success → bounded escalation. Local-first is preferred only when capability and verification evidence pass. | Matrix chooses cheapest passing candidate; ungraduated coordinator, missing auth, stale probe, or missing success evidence is denied or escalated. |
| C4.3 | C4.2 | Wire surgical fixes through existing router/reservation/ledger/projection seams; never infer billing from binary/provider name. | Focused cost/reservation/projection tests and the inherited B5 accounting gates pass; duplicate child roll-up and concurrent claims remain exactly once. |
| C4.4 | C4.3 | Journey: local candidate fails a verifier → bounded cheap fallback → stronger route only when evidence warrants; UI projection and CLI announcement show route, cost class, estimate/unknown, and terminal result. | Paired treatment/control report proves USD/verified-success and latency deltas; no runtime pass assumed from a matrix-only result. |
| C4.5 | C4.4 | Independent cost/security verifier audits authorization before dispatch, post-dispatch unknown handling, estimates, and rollback. | `C4-routing-receipt.json` links raw ledger rows, route decisions, verifier results, confidence/limits, and exact spend approval. |

**Consequence/security/rollback:** Mispriced local work hides cost; cheap but
unverified work creates false success; fallback storms can burn credits. Roll
back route policy to the last measured matrix, park unknown billing, and retain
the over-reserved hold until reconciled as required by B5/B6.

**Bounded retry:** one route-policy change per frozen slice; at most one fallback
per node and one verifier. Broad paid benchmarks require a new explicit cap.

<a id="c5-bounded-verification"></a>

## C5 — Bounded verification

**Purpose:** spend verification effort in proportion to impact while refusing
vacuous passes, stale evidence, and under-verified high-risk effects.

**Dependencies:** C4.5. **Owner/model policy:** deterministic local checks first;
  cheap verifier for low-risk work; independent high-reliability verifier for
  security, data, cross-platform, paid, or irreversible effects; human approval
  only for existing release/high-impact/threshold gates.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed: `crates/goose/src/goal_refinement.rs`,
  `agents/platform_extensions/developer/verify.rs`, `gate_classifier.rs`,
  `execution_receipt.rs`, `publish_sequence.rs`, `dispatch_scope.rs`, and
  `after_turn.rs`.
- Confirmed test/evidence targets: `crates/goose-server/tests/growth_verify_completes.rs`,
  `decision_wiring.rs`, `trust_boundary.rs`, and `crates/permagent-eval` qualification
  code. Exact risk labels used by every effect are **unconfirmed** and must be
  audited before changing policy.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and integration gate |
|---|---|---|---|
| C5.1 | C4 | Inventory effect risk: read-only/mechanical, bounded code edit, cross-file, auth/data, external/paid, irreversible/release. Map risk to required checks and verifier independence. | Every code goal has a runnable acceptance check; absent/invalid checks are `uncertain`, never pass. |
| C5.2 | C5.1 | Freeze no-model fixtures for vacuous success, stale evidence, changed-input mismatch, timeout/cancel, verification-loop cap, and low/high-risk proportionality. | Same failing gate is not repeated unchanged; rework budget/parking is durable and visible. |
| C5.3 | C5.2 | Implement only missing risk gates or evidence binding at existing verify/receipt/publish seams. | Focused goal-refinement, receipt, trust, and qualification tests pass; successful low-risk work is not forced through expensive Council, while high-risk work cannot bypass independent review. |
| C5.4 | C5.3 | Journey: low-risk mechanical task lands after deterministic check; cross-platform voice/state change requires focused + integration + fresh UI/device evidence; a failed verifier parks after cap. | Command Center/run projection exposes verification depth and terminal state; no “green” UI without receipt hash. |
| C5.5 | C5.4 | Independent verifier reviews risk classification, evidence freshness/provenance, rollback rehearsal, and human-gate boundaries. | `C5-verification-receipt.json` records risk, required/actual checks, attempts, evidence hashes, and any blocked hardware/environment gate. |

**Consequence/security/rollback:** Under-verification can land unsafe or broken
effects; over-verification burns time/cost and hides real blockers. Roll back
only the policy/receipt change, preserve the failing artifact, and re-open the
earliest owning node rather than relabelling a failed check.

**Bounded retry:** one changed implementation/diagnostic plus one independent
verification; a cap parks with the best evidence and minimum operator decision.

<a id="c6-plan-and-trust-consistency"></a>

## C6 — Plan and trust consistency

**Purpose:** make the approved plan executable, the live status truthful, and
the documentation/receipts agree without introducing a parallel controller.

**Dependencies:** C5.5. **Owner/model policy:** deterministic schema/graph
validator first; cheapest documentation worker for mechanical cross-links;
strong integrator only for plan/status conflicts; human approval remains only
where existing policy requires it.

**Surgical targets (confirmed unless marked otherwise)**

- Confirmed plan/dispatch: `crates/goose/src/council/deliver.rs`,
  `agents/platform_extensions/orchestrator.rs`, `goal_engine.rs`,
  `dispatch_brief.rs`, `council/verdict.rs`, and `scheduler.rs`/
  `scheduler_trait.rs`.
- Confirmed status/projection: `crates/goose-server/src/routes/runs.rs`,
  `routes/henry_status.rs`, `routes/status.rs`, `session/budget_projection.rs`,
  `ui/command-center/src/lib/costMeter.ts`, `ChatApp.tsx`, and status/run
  components. Exact route-to-component bindings are **unconfirmed** until C6.1.
- Confirmed documentation authorities: the master/excellence/task-budget DAGs,
  voice master DAGs, and this companion. No prose status may override a typed
  receipt or controller state.

**Sequential child DAG**

| ID | Depends on | Required action | Acceptance/regression and integration gate |
|---|---|---|---|
| C6.1 | C5 | Trace approved plan hash → durable node/card → dispatch → child evidence → verifier → status projection → document receipt. Identify every label that can drift. | One graph shows authoritative owner for each field; invalid/cyclic/missing-dependency plans are rejected before spend. |
| C6.2 | C6.1 | Define consistency invariants: active/ready/blocked frontier, dependency order, status freshness, receipt hash, budget projection, route/model, retry count, and doc link. | No “implemented” or “passed” status without required evidence; inherited B5/B6 remain represented exactly as they are. |
| C6.3 | C6.2 | Add deterministic source/schema/document checks at existing validators and status projections only. | Plan validator, run/status route tests, budget projection tests, and docs link/anchor checks pass; no second controller or status database appears. |
| C6.4 | C6.3 | Journey: approve a multi-node plan → execute one dependency chain → inject a blocked child → resume after correction → expose status in Command Center and CLI → produce final receipt. | UI, API, CLI, plan, and this document agree on node IDs, dependencies, terminal state, cost/evidence; stale status is marked stale/unknown. |
| C6.4a | C6.4 | Inventory existing identity and operator trust fields, then expose an evidence-derived capability-specific trust history using existing Spectral records and UI patterns. Include operator correction/revocation, sparse evidence, failures and version changes. If no compatible identity extension exists, report the gap before designing one. | New regressions reject forged/duplicate self-awarded success and cross-operator mixing; revocation survives restart and device reconnect; reputation cannot grant permission. The operator can inspect evidence and correct trust in the UI. No public score publication or blockchain changes. |
| C6.5 | C6.4a | Independent integrator performs a read-only consistency audit and publishes the final companion index. | `C6-consistency-receipt.json` contains graph hash, projection snapshots, document links, commands, raw evidence, rollback reference, and unresolved items. |

**Consequence/security/rollback:** Inconsistent status can trigger duplicate
dispatch, conceal an active worker, misstate spend, or cause an operator to
approve the wrong change. Roll back the projection/document adapter to the last
versioned contract; never “fix” a mismatch by editing a status label or deleting
the underlying receipt.

**Bounded retry:** one schema/adapter correction and one integrator audit. A
plan/status/document mismatch that persists is blocked and routed to the
existing Decision Inbox; it does not create a parallel orchestrator.

## Terminal qualification and handoff

The companion program qualifies only when C0 through C6 each have fresh passing receipts,
all required focused regression commands pass, and the integrated journeys preserve permission, budget, provenance,
platform state, and rollback invariants. The final handoff must link:

1. six node receipts and their source/build hashes;
2. inherited B5/B6 receipts without changing their status;
3. raw test/log/UI/device evidence and unavailable hardware declarations;
4. route/model/billing and measured verified-success comparisons;
5. Spectral recognition/provenance IDs and operator-correction audit trail;
6. rollback commands and the smallest unresolved human decision.

Environment-blocked or unavailable-device results remain incomplete gates, not
qualification passes. C7 in the master additionally requires exact-build device
journeys, inherited qualification and final operator release approval.

No production edit is authorized by this document alone. The existing
orchestrator/controller must schedule each approved node only after its
dependencies and gates are satisfied.
