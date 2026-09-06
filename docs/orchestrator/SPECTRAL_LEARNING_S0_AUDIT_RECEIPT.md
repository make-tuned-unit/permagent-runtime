# Spectral learning S0 audit receipt

**Date:** 2026-09-05  
**Status:** S0 evidence audit passed after root review; S1 active, implementation qualification pending  
**Owner:** Spectral learning S0 (`/root/spectral_learning_audit`)  
**Owned file:** this receipt only

This is a bounded, read-only source audit. No production source, Spectral
checkout, provider, private-data store, or database was changed. No runtime
dispatch, daemon start, test, benchmark, provider call, or goal dispatch is
claimed. Existing dirty worktree changes were preserved.

Root reviewed the pinned API/source map and explicit unknowns on 2026-09-05,
confirmed reuse of existing recognition instrumentation, and advanced the
audit gate only. Ambient activity retention policy and recognition opt-in are
distinct; the ordering observation below is not by itself proof of a consent
breach. S1 adversarial review and executed regressions are required before
episode wiring is promoted. No product/runtime gate passes with S0.

## 1. Evidence boundary

The actual workspace dependency is:

```toml
Cargo.toml:88
spectral = { git = "https://github.com/make-tuned-unit/spectral",
             rev = "7025328e198a2df4826f5c4aba173162d16cc521" }
```

`Cargo.lock:10071-10082` repeats the same revision. The cached checkout at
`/Users/j/.cargo/git/checkouts/spectral-121c60948af2c3d3/7025328` resolves to
that revision. The sibling checkout at `/Users/j/Documents/dev/spectral` was
read only and is at `a0320e6`; it is not the workspace pin and is not evidence
for current Permagent behavior.

`docs/architecture/SPECTRAL_INTEGRATION.md:3-12,60-65` is stale: it names
`e9a80d8`/`5b9c457f` and says `RecognitionContext.session_id` wiring is pending.
The current source has session-context plumbing in the recall producer. Some
comments in `crates/goose/src/recognition_sink.rs:13-33,59-64` also name the
older `c2c8381` despite the Cargo pin above. These documents/comments must not
override the locked dependency when S1 contracts are written.

The pinned Spectral API was inspected directly:

| API/source | Confirmed capability | S0 limitation |
|---|---|---|
| `spectral/src/lib.rs:115-122,350-403,524-575` | `remember_with`, `recall`, `forget`, `vacuum`, `recognize`, `reinforce`, `probe` and `recall_cascade` are exported | Presence of an export is not runtime wiring or a passing test |
| `spectral/src/turn.rs:141-182,234-276,302-475` | Turn requests accept explicit observations; `Used` is the only reinforcing outcome; receipt/outcome are keyed and idempotent | Permagent samples this path; no general turn outcome is established |
| `spectral-recognition/src/stream.rs:23-65,109-224` | Fixed `Cue`, enrolled `Segment`, mutable `StreamTracker`, and lock events exist | Permagent has no segment mining/storage/versioning or tracker owner/lifecycle |
| `spectral-cascade/src/context.rs:6-100` | Context carries recent activity, focus wing, persona and session ID | Permagent currently supplies only a subset on each path; persona is not an operator identity |

The documented 12.9 ms median / 73 ms p90 / 86 ms p99 recognition figures
(`recognition_sink.rs:90-109`) and any historical AUC statement are inherited
source comments, not measurements made in this audit.

## 2. Executive finding

Spectral is the existing durable memory and recall substrate. Permagent does
have a local `permagent.db` recognition-instrumentation sidecar, but it stores
retrieval evidence and joins rather than a second memory or reputation store
(`session/spectral_schema.rs:1172-1239`). S1 must decide the compatible contract
for that evidence before any learning implementation; no parallel memory worker
or model-weight training system is justified.

The current state is mixed:

* Query-mode recognition is wired alongside the daemon's main cascade recall
  when the session pool is present (`brain_ops.rs:284-358`). It is detached,
  bounded, best-effort, and can be dropped without failing the reply
  (`recognition_sink.rs:296-398`). This confirms a source seam, not a runtime
  pass or complete learning loop.
* Ambient consent plumbing is wired only to the sink boundary. Activity is
  written to Spectral before `observe_ambient_cue` checks consent
  (`activity/ingestion.rs:317-342,432-442`); the current ambient function only
  logs an allowed cue (`recognition_sink.rs:401-432`). `StreamTracker` is not
  owned or fed. Thus “consented ambient feedback” is not a live tracker or
  feedback loop.
* Recall evidence distinguishes the whole retrieved set from exact injected
  memories and later writes a lexical citation outcome
  (`brain_ops.rs:309-341`, `recognition.rs:247-328`). This is useful
  instrumentation, not validated usefulness, causal success, or verified
  preference learning.
* Spectral `Brain::turn` outcome reporting is sampled and defaults off
  (`turn_sampling.rs:1-120`); sampled outcomes are detached and use the same
  exact-content citation proxy (`brain_ops.rs:150-200`).
* There is no durable operator identity, provider/model/version snapshot, or
  physical invocation identity on a recognition event. `rc_persona` is filled
  from the placeholder default persona (`state.rs:1109-1135`,
  `config/agent_identity.rs:63-83`), and the pinned Spectral persona field is
  not an operator-specific recall mechanism. One-model and many-model
  adaptation therefore remain unproven.

## 3. Confirmed event → memory → recall → outcome seams

| Stage | Confirmed source behavior | Present data | Missing, dropped, or ambiguous |
|---|---|---|---|
| Activity event | `ActivityIngester::handle_event_blocking` routes Always/Aggregated events; pause suppresses Brain writes, while Ephemeral events are not written (`activity/ingestion.rs:222-245`) | Filtering, browser-domain dedup, active-project wing, source/device, private visibility, raw compaction tier, optional `event.session_id` as `episode_id` (`:281-342`) | Consent is not the write gate. Rendered activity can include command/URL/title/form or selected text (`:613-748`). No explicit operator scope, expected outcome, or correction link |
| Activity → memory | `remember_with` writes to Spectral; inserted/recurrent outcomes emit/reinforce (`activity/ingestion.rs:317-430`) | Spectral memory ID/key/content/source/device/confidence/wing/episode and timestamps, per pinned ingest structs | No recognition segment enrollment. The later consent seam receives only wing/source/type/content length, not a durable activity or memory ID |
| Chat recall | Server builds `RecognitionContext`, calls `recall_cascade`, filters score floor 0.7/top three, and injects typed provenance (`brain_ops.rs:209-402`) | Query, session ID, focus wing, placeholder persona, retrieved IDs/scores/ranks, exact injected IDs, prompt contribution IDs | The activity context-builder path calls cascade with an empty context plus placeholder persona and bypasses `brain_ops::inject_recall` (`activity/context_builder.rs:157-195`); it therefore does not create the normal recognition row |
| Recall instrumentation | `spawn_persist_recognition` records one retrieval and the whole set; injected IDs are separately recorded (`session/spectral_schema.rs:1172-1239`, `brain_ops.rs:309-341`) | `retrieval_id`, session/query/time, strategy, persona/context fields, set members, injected IDs/source | Raw query is persisted. No provider/model/version, operator ID, physical call ID, selection/acted/verified states, expected outcome, or correction provenance |
| Recognition | Detached `SafeBrain::recognize` resolves recognized IDs, degrades unresolved IDs to `Familiar`, then writes verdict/familiarity if a handle exists (`recognition_sink.rs:315-398`) | Verdict, familiarity, stimulus, wing, session, retrieval ID; orphan/unresolved is observable in logs | No proof that detached work completes in a running daemon; timeout/error means dropped observation. Recognition is not a usefulness label |
| Reply outcome | Lexical five-word overlap finds cited injected memories; task/decision paths can write outcome labels (`recognition.rs:247-379`) | Cited IDs, checked timestamp, positive/negative outcome label, source/kind/polarity where exposed | Exact lexical overlap misses paraphrase and cannot prove that a citation helped. Silence, interruption, rescue, rejection, or a model switch are not cleanly separated |
| Spectral turn outcome | Sampled turn reports every delivered hit as `Used` on lexical cite, otherwise `Ignored`; only `Used` reinforces (`brain_ops.rs:150-200`, pinned `turn.rs:417-475`) | Receipt, delivered memory IDs, Used/Ignored outcomes | Default sample rate is 0; no general expected/observed action outcome or verified task completion |
| UI/CLI consumers | UI receives `ContextAttached` for the ambient context digest and `/api/brain/search` returns search fields; CLI uses loopback brain search and detached turn persistence (`routes/reply.rs:374-425`, `routes/brain.rs:106-241`, `goose-cli/session/brain_sync.rs:61-350`) | UI citation marker, search ID/preview/score/source/timestamp; CLI recall text and typed historical hint | No retrieval ID, verdict/familiarity, cited/ignored/wrong state, correction, model attribution, or inspect/correct/forget/revoke learning control reaches the UI. CLI write request has session/turn/text/workdir but no learning join IDs |

### Observation versus inference

Observed in source: a memory was retrieved, a subset was injected, a detached
recognition call was scheduled, a lexical overlap was found, or a task/decision
row was written. These are event facts only.

Not established and must not be labeled as facts: “retrieved means useful,”
“cited means correct,” “Familiar means this operator knows it,” “no outcome
means ignored,” “operator rescue means model failure,” “active project means
operator preference,” or “placeholder persona means operator identity.” A
missing row can mean no terminal event, no pool/feature, a dropped detached
write, or an unobserved path; absence and failure need separate states.

## 4. Field and authority gaps for S1

S1 should define linked, versioned events before wiring any learner. The common
contract in `SPECTRAL_LEARNING_SUBDAGS.md:75-92` requires at least operator
scope, session/task/turn, physical invocation, model/version, contribution,
recalled memory/version, action, expected/observed outcome, explicit correction,
and uncertainty. Current code has only a subset:

* Present or joinable: session ID, turn index, wing/focus, memory ID/key,
  score/rank, visibility, source/device, confidence, episode ID, retrieval ID,
  query, injected IDs, citation IDs, task/decision labels, recognition verdict
  and familiarity.
* Absent or dropped: opaque operator identity/scope, provider/model/version and
  route, physical invocation/call ID, contribution-to-action link, memory
  version, expected outcome, observed verified outcome, explicit correction
  event, correction precedence, selection-versus-injection-versus-action
  versus-verification, stream segment/tracker/owner, and cross-device causal
  ordering. The actual provider/model exists elsewhere in session routing but
  is not snapshotted into `recognition_events`.
* Privacy/authority constraints: query text and activity-rendered content can
  be private; local-only recognition is not an authorization to infer identity
  or preference. Learning may not grant permissions or alter C4 routing by
  reputation. Cross-operator isolation, deletion/derivative invalidation, and
  orphan recognition rows require explicit fixtures. `recognition_sink.rs:50-71`
  documents the separate recognition DB/orphan hazard; raw delete paths must
  not be treated as sufficient forgetting.

The consent module is an all-off, per-wing/per-source choke point
(`recognition_consent.rs:23-30,85-117`), but it gates only ambient cue delivery
to the current sink. It does not stop the activity memory write, does not own
stream lifecycle, and has no demonstrated UI journey for revocation or
replay-safe derivative deletion. Query mode is deliberately not consent-gated
according to `recognition_sink.rs:82-88`; S1 must make that local instrumentation
policy explicit rather than silently treating it as ambient consent.

## 5. Synthetic baseline cases to freeze (not executed)

These are the smallest deterministic fixtures for S1-S4. They are proposed
cases, not passes:

1. Retrieved + selected + injected + exact reply overlap: record the current
   lexical `Used` proxy, while keeping “useful/correct” unverified.
2. Retrieved but filtered out: exposure only; it must not receive an injected
   or acted-upon label.
3. Injected paraphrase with no exact five-word overlap: demonstrate current
   false `Ignored`/unknown behavior before changing the detector.
4. Explicit correction followed by stale retrieval: correction precedence,
   deletion and derived-lesson invalidation must beat familiarity/reinforcement.
5. Wrong, rejected, interrupted, rescued, cancelled, and no-terminal-outcome
   actions: keep each state distinct; silence is not approval.
6. Duplicate/reordered delivery, restart, missing session pool, recognition
   timeout/error, orphan memory ID, consent revoke/replay, cross-operator ID,
   model/provider/version change, and context truncation.

Unknown today: recall precision/usefulness, repeated-correction burden, false
carryover, correction latency, p95/p99 on this revision, detached event drop
rate, storage/compute cost, model attribution quality, and operator isolation.
No baseline should be filled with a source comment or a symbol-presence
inference.

## 6. Exact next-node targets and focused commands

No command below was run in S0. They are bounded candidates to be reviewed by
root and the node owner; they are not runtime evidence.

| Node | Smallest target | Regression/verification candidate |
|---|---|---|
| S1 contracts | Add synthetic contract fixtures around `recognition.rs`, `session/spectral_schema.rs` and `context_packet.rs`; decide whether evidence remains the existing sidecar or needs the minimal compatible Spectral API change | `cargo test -p permagent --lib recognition::tests`; `cargo test -p permagent --lib recognition_consent::tests`; inspect schema migration tests; no new store |
| S2 episodes | Wire only established IDs at `brain_ops.rs`, `routes/reply.rs`, `session_events.rs`, `activity/ingestion.rs`, and CLI `brain_sync.rs`; define queue overflow/restart/unavailable states | `cargo test -p permagent --lib recognition`; focused daemon route tests selected after test-name discovery; duplicate/reordered/orphan fixtures |
| S3 feedback | Correction and consent fixtures around `recognition.rs` and `recognition_consent.rs`; find actual task/decision correction API before claiming a callsite | `cargo test -p permagent --lib recognition_consent::tests`; targeted correction/outcome tests; replay-after-revoke and silence/interruption/rescue cases |
| S4 recall | Preserve retrieved/selected/injected/acted/verified states in the existing recall seam; address sampled `turn` and ambient-context bypass separately | `cargo test -p permagent --lib turn_sampling::tests`; focused brain-ops/recognition tests for exact detail, contradiction, abstention, deletion and truncation |
| S5 adaptation | Attribute evidence to existing session route/model identities without changing C4 route enforcement or B5/B6 accounting; shadow only | C4-owned routing qualification plus a no-model held-out fixture; no latent/model-weight training |
| S6 UI | Extend the existing backend event/search contract and command-center consumers; expose source/limits and inspect/correct/forget/revoke only after S1-S4 IDs are stable | Existing focused UI type/build checks and an exact supported-device journey; screenshots alone are insufficient |
| S7 qualification | Controlled memory-disabled/current-policy/treatment fixtures under fixed budgets; preserve revision, model, limits and no-run reasons | Exact-build, rollback, privacy and longitudinal held-out review; human release gate |

## 7. Collision and ownership matrix

* **Companion C3:** owns the surrounding recognition/worker briefs and
  consumes S1-S4 contracts. S1-S4 own memory semantics and must not create a
  second recognition implementation. The current `recognition.rs`, sink,
  consent and schema changes need one named owner before mutation.
* **Companion C4:** owns provider/model routing enforcement. S5 may supply
  attributed evidence and shadow proposals; it must not edit route authority,
  permissions, or spend behavior.
* **Companion C6:** owns trust/identity projection and operator controls. S6
  may provide evidence and UI contracts; learning cannot turn familiarity or
  satisfaction into permission or a universal profile.
* **Companion C2:** shares platform/UI adapters. Backend `ContextAttached`,
  brain search, and command-center consumers require one production-path owner
  and one receipt for a shared change.
* **Harness B5/B6:** no Spectral learning node edits cost reservations,
  projection, accounting, or active B5/B6 paths. Any model evidence must use
  existing invocation/budget identities and reopen the owning gate on conflict.

## 8. S0 gate decision

**Pass for audit evidence map only. Blocked for implementation/promotion until
S1 contracts are reviewed.** The evidence establishes the existing Spectral
memory/recall substrate, a query recognition observer, an ambient consent
choke-point-only seam, and a partial usefulness proxy. It does not establish a
live ambient tracker, complete consented feedback, operator-specific learning,
one/many-model adaptation, runtime success, UI truth, or regression coverage.

The smallest safe continuation is S1 synthetic contracts and missing-field
fixtures. Do not activate `StreamTracker`, infer operator/model identity,
reinforce from citation alone, or claim a dispatched learning goal until those
contracts and their focused regressions exist.
