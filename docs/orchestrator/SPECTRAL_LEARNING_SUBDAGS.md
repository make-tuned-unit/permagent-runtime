# Spectral operator-learning program

Created 2026-09-05. S0 audit active; implementation and qualification not yet
passed. The schema-1 master manifest uses the existing `permagent-eval program`
controller. A valid manifest is not proof of runtime dispatch. Existing approved
roadmaps schedule execution; Spectral and existing session records remain the
durable substrate. No new framework, scheduler, parallel memory, public reputation
service or model-weight training system is introduced.

## Parallel execution and shared ownership

This program runs alongside `PERMAGENT_COMPANION_MASTER_PROGRAM_DAG.yaml`.
Read-only audits and disjoint fixtures may overlap immediately. One worker owns
each production path at a time; root assigns paths before implementation and
records source revision plus dirty-diff identity. Existing edits are preserved.

| Shared area | Coordination rule |
|---|---|
| Companion C3 recognition/worker briefs | S1-S4 own memory semantics; C3 consumes their contract and receipts, not a second implementation |
| Companion C4 model routing | S5 supplies attributed learning evidence; C4 owns routing enforcement and qualification |
| Companion C6 trust/identity | S6 supplies operator control and evidence; C6 owns permission-independent trust projection |
| Companion C2 UI/platform state | Shared adapters get a single named owner; both programs require compatible event/UI evidence |
| Harness B5/B6 accounting | No memory worker edits their active paths without handoff; model work uses existing invocation/budget identities |

Cross-program constraints are explicit entry gates checked by the integrator,
not automatically enforced by the schema-1 graph validator. Block only the
conflicting mutation node; continue safe independent preparation. A shared change
must attach one receipt to both owners and reopen affected gates on regression.
Do not promote one program by merely reading a passed status in the other.

## Common node contract

Every node includes actual allowed files, dependencies, current Spectral API,
source/provenance, smallest edit, new regression test, affected API/UI consumers,
side effects, rollback, named worker/model, bounded spend/time and verifier.
Sequence: reproduce -> regression -> surgical implementation -> focused tests
-> adjacent integration -> UI journey -> receipt. Production changes require new
or extended regressions. Internal helpers may land behind existing controls, but
no user-facing feature passes while its UI or rollback is missing.

Use low-cost qualified workers; root plans/reviews. Prefer no-model fixtures and
local inference where realistic. No cloud disclosure from local-only policy and
no silent paid fallback. Live trials need an approved numeric cap; broad consent
to paid models is not an invented unlimited trial budget. Two unsuccessful repair
attempts trigger root diagnosis/replanning, not identical retries or routine human
approval. Use events/completion notifications, never continuous status polling.

Observe only consented in-scope app/agent activities. This is not permission for
OS-wide surveillance, background audio capture or recording every keystroke.
Keep opaque provider state confined to its supported continuation purpose; do
not treat signatures as readable thoughts or cross-provider semantic vectors.
Exclude raw private contents and secrets from public receipts. Minimize retention,
preserve deletion/revocation and distinguish absence of evidence from failure.

## S0 Audit

S0.1 -> S0.2 -> S0.3

1. Inspect actual Cargo pin/patch and the corresponding Spectral library API,
   not historical documentation alone. Inspect recognition.rs, recognition_sink.rs,
   recognition_consent.rs, CLI session/brain_sync.rs and actual daemon producers.
   Read sibling Spectral source if accessible; edits outside the writable repo
   require the appropriate permission and separate owner.
2. Trace activities, provider metadata, context actually injected, recall,
   corrections, ambient event consent, outcomes and platform consumers. Record
   where data is dropped, duplicated, inaccessible or merely unimplemented. Check
   blocking/async behavior and recognition tracker lifecycle explicitly.
3. Freeze synthetic baseline cases and ownership matrix. Measure existing detail
   recall, correction precedence, repeated mistakes, latency and processing cost
   where possible; unknown baselines remain unknown. Record concrete package/test
   commands only after discovery. Root reviews `SPECTRAL_LEARNING_S0_AUDIT_RECEIPT.md`.

Gate: actual API/source map and evidence gaps agreed; no inferred runtime passes.

## S1 Contracts

S1.1 inventory reuse -> S1.2 fixtures -> S1.3 privacy/integration review

Define linked episode semantics using existing IDs: operator scope, session/task,
turn, physical invocation, model/version, contribution, recalled memory/version,
action, expected outcome, observed outcome, explicit correction and uncertainty.
Do not require a model-generated prediction for every trivial action.

Separate observation, interpretation, preference, hypothesis and verified lesson.
Specify correction precedence, validity conditions, retention, consent withdrawal,
deletion and derivative invalidation. Cross-device event ordering must not depend
only on wall-clock time. Proposed numeric scores remain estimates with provenance.
If current APIs lack a capability, specify the minimal compatible Spectral change
before editing rather than inventing a side store.

Gate: schema/contract fixtures for missing fields, privacy boundaries, version
compatibility and rollback; no model-generated interpretation masquerades as fact.

## S2 Episodes

S2.1 failing join fixtures -> S2.2 existing-producer wiring -> S2.3 recovery tests

Wire contribution/activity/feedback references at existing producers and Spectral
adapters. Preserve actual model route and observed outcome separately from intended
route and expected result. Bound queues, define overflow behavior and backpressure;
record capture gaps instead of silently claiming complete episodes.

Regressions: duplicate/reordered delivery, cancellation, missing outcome, restart,
cross-operator isolation, orphan IDs, version change and partial persistence failure.
Use synthetic data and temporary stores. Gate includes event/API consumer contract
tests and explicit unavailable state. Rollback disables new capture through existing
configuration without destroying prior valid memories.

## S3 Feedback

S3.1 consent fixtures -> S3.2 capture adapters -> S3.3 interpretation gate

Capture explicit corrections first; then consented edits, undo, re-query, action
acceptance and tool/environment results where actual interfaces expose them.
Attribute artifact diffs to the source proposal only when identity is established.
Do not label a model switch a preference without contextual evidence.

Tests: silence is not approval; interruption is not dislike; operator rescue is
not model success; rejected action does not infer a global preference. Verify
consent UI/API toggles stop collection and replay cannot restore revoked consent.
No personality/emotion inference from voice prosody as a default capture feature.
Use deterministic event processing first; interpretation calls are bounded and
optional, with source-linked uncertainty. Gate: end-to-end feedback provenance,
retention and permission tests plus working operator controls.

## S4 Recall

S4.1 baseline recall fixtures -> S4.2 actual-context attribution -> S4.3 outcome link

Distinguish retrieved, selected, injected, acted upon and subsequently verified
memory. Mere retrieval frequency does not establish truth or causal usefulness.
Use existing recognition patterns to find related experience, preserving negative
and contradictory evidence instead of reinforcing familiar claims automatically.

Tests: exact detail, temporal scope, corrections, contradiction, absence/abstention,
deletion including derived lessons, stale-memory invalidation, prompt injection,
cross-user leakage and retrieval under context truncation. Recognition tracker
ambient mode requires explicit lifecycle/consent/resource limits before activation.
Gate: precision and usefulness assessed separately; no unsupported causal claim.
Rollback restores prior selection policy without erasing correction provenance.

## S5 Adaptation

S5.1 frozen policy baseline -> S5.2 shadow proposals -> S5.3 bounded activation

Adapt briefs, clarification strategy, context selection and review recruitment
through existing policy/role mechanisms. One model must work without a Council.
Many models contribute lazily; adding configured models does not activate them all.
Latent communication and model-weight training remain separate future research.

Test single/multi-model parity, unavailable workers, version drift, cold start,
misattribution, adversarial feedback, preference change and permissions invariant.
Separate satisfaction, correctness, safety and latency metrics. Shadow comparisons
and held-out tasks must test whether the lesson improves behavior; record uncertainty
and negative transfer. Retain a reversible policy version and operator override.

Gate: predeclared improvement criterion met without safety/privacy regression;
no overfitting or success credit from the agent grading its own unsupported claims.
Coordinate routing changes with Companion C4 before mutation.

## S6 Operator UI

S6.1 consumer contract -> S6.2 platform wiring -> S6.3 device/reconnect journeys

Using existing UI components, expose what was learned, its source and limitations,
and inspect/correct/forget/revoke actions. Support contextual preferences rather
than one universal operator profile. Never portray tentative inference as identity.
macOS and iOS expose supported controls; watchOS uses a clear supported handoff
where full inspection is unsuitable. Preserve WatchConnectivity and existing auth.

Tests: accessible controls, API errors visible, partial states honest, revoke and
correction survive restart, offline queue cannot resurrect deleted knowledge,
duplicate handoff has no duplicate effect, device scope and operator identity match.
Gate: exact-build evidence for supported journeys; screenshots alone do not prove
durable action. Coordinate with Companion C2/C6; no duplicate UI truth store.

## S7 Qualification

S7.1 integrated adversarial suite -> S7.2 longitudinal held-out evaluation
-> S7.3 exact-build/rollback review -> operator release approval

Evaluate repeated synthetic operator interactions and preference shifts, comparing
memory-disabled/current-policy/treatment under controlled budgets. Measure repeated
correction burden, exact-detail recall, inappropriate carryover, verified completion,
latency, storage/compute cost, privacy failures and operator ability to reverse
learning. Short smokes diagnose but do not establish lifelong improvement.

Retain dataset/revision, actual models, limits, source/build identifiers, raw test
results and no-run reasons in existing evidence records. Infrastructure failure
and missing hardware remain incomplete gates, never passes. Reopen the earliest
owning node on regression and invalidate affected downstream receipts.

Final gate: all required integration/device evidence, original program qualification
requirements where shared, reversible deployment and clear operator consequences.
No new cloud service, public reputation or Spectral repository modification is
implicitly authorized by this manifest. Release remains an explicit human gate.
