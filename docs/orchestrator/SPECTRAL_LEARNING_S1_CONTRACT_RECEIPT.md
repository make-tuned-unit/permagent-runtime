# Spectral learning S1 receipt — recognition contracts

Date: 2026-09-05 (America/Halifax)

Status: focused contract and recognition-writer tests executed successfully;
full downstream learning/privacy/device acceptance remains pending.

Latest executed results (see `END_TO_END_RUNTIME_GATE_RECOVERY.md`, focused
post-phone gate results): `recognition_contract` **9 passed**, and
`recognition::tests` **25 passed**, both Cargo exit 0. These supersede the
historical compile-only/pending notes below; they do not promote all S1/S2
requirements or imply that S3 learning is implemented. The bounded
retention/withdrawal follow-up tests documented below were added after that
execution and still require the shared Rust verification slot.

## Scope

Added the pure `recognition-contract.v1` envelope in
`crates/goose/src/recognition_contract.rs`, reusing existing recognition IDs
without adding a memory store, tracker, learner, daemon analytics path, or
SessionManager budget code. The contract keeps these facts distinct:

- observation, interpretation, preference, hypothesis, and verified lesson;
- retrieved, selected, injected, acted, and verified stages;
- query instrumentation versus ambient recognition consent;
- partial/missing operator, session, invocation, provider/model/version,
  contribution, retrieval, memory, outcome, and correction metadata;
- correction precedence only when the same non-empty operator and target have
  an explicit `supersedes` edge at a newer causal revision; concurrent and
  cross-operator corrections remain unresolved, regardless of wall-clock or
  identifier ordering;
- bounded uncertainty scores with an explicit basis and timestamp.

Activity capture is represented separately from ambient recognition consent.
`CaptureBoundary` delegates ambient admission to the existing
`ambient_cue_allowed_with` policy; an activity row neither grants consent nor
is required before an ephemeral consented cue can be recognized. The
serialized capture-time consent block is audit metadata only; ambient records
must use `validate_with_current_consent` with the trusted live config, so a
revoked policy cannot be replayed from an old envelope.

Completeness is scoped to the claim kind and evidence stage: sparse retrieval
observations remain sparse/valid, while interpretations require their identity
and model join fields, and verified lessons require a non-empty observed
outcome plus real source-event/derivation evidence and the `Verified` stage;
lessons do not require a correction unless they are actually correcting an
earlier fact. Attached corrections must match the record operator and exact
memory target (or record ID when no memory ID exists). Existing
`PendingRecognition` and `RecognitionSeen` are adapted directly so this
contract is an adapter over the durable recognition substrate rather than a
detached memory schema.

## Synthetic regression coverage

The module fixtures cover observation-versus-interpretation separation, sparse
identity/metadata remaining partial, activity capture without ambient consent
being rejected, explicit ambient opt-in, correction precedence across skewed
device clocks, cross-operator/equal-revision/conflicting-target rejection,
malformed timestamps and whitespace metadata, verified-lesson evidence, and
invalid/non-finite uncertainty. Revoked-after-capture replay is rejected,
attached corrections are bound to operator and target identity, and a
verified lesson is not required to be a correction. The `RecognitionSeen`
adapter fixture also preserves missing outcomes as unknown.

## Retention and withdrawal boundary

The existing substrate has two deliberately separate operations:

* `recognition::prune_recognition_instrumentation` removes expired
  `recognition_events` and `recognition_tool_events`; the foreign-key cascade
  removes their retrieved-set members. The focused regression also checks that
  a second pass is idempotent and leaves fresh evidence intact.
* Spectral's existing `Brain::forget(key)` / Permagent's `SafeBrain::forget`
  hard-delete one explicit memory key across the memory and recognition
  substrates and return a verified deletion report. This is the established
  deletion API; it is not invoked by consent configuration changes.

`recognition_consent::store` currently persists policy only. There is no
withdrawal handler that can safely derive a set of ambient memory keys,
recognition rows, or derived lessons from a policy change. Therefore this S1
slice does **not** claim that revoking ambient consent deletes unrelated
operator memories or invalidates derivatives. Doing so would require an
explicit identity/retention contract and a compatible producer/API change;
blanket deletion would be incorrect for query-mode instrumentation and for
memories outside the revoked wing. The contract regression covers the current
boundary: revoked ambient envelopes cannot be replayed, while explicit
query-mode evidence remains governed by its separate local-instrumentation
policy.

The focused follow-up tests added for this boundary are pending the shared
runtime Rust execution slot; no production producer, store, or deletion path
was changed.

## Verification boundary

The bounded compile command was started once before the shared build-slot
coordination constraint was reiterated:

```text
cargo test -p permagent --lib recognition_contract --no-run
```

At that initial checkpoint no completed Rust result existed; the focused
executed results above now supersede it. Further Rust execution must be
scheduled through `close_runtime_gate`. No provider calls,
daemon analytics, SessionManager budget tests, or Spectral repository edits
were made. The bounded S2 follow-on now records only deduplicated physical
invocation-ID references on the existing recognition row after durable cost
settlement; provider/model facts remain resolved from the authoritative
`cost_ledger` by session-checked readers. Missing, cross-session, malformed,
and overflow cases remain explicit unavailable/partial states.

The guarded existing-table additions are `provider_invocation_ids` (bounded
JSON references), `attribution_status`, and `attribution_observed_at` on
`recognition_events`; no new table was introduced.

The additive `Agent::reply_with_recognition` entry point carries the existing
retrieval ID from the recall-aware server route while preserving all existing
`Agent::reply` callers. No global current-recall state, timestamp join, raw
content logging, model call, or operator/model identity synthesis was added.

S2 producer ordering is bounded: the recall-aware route waits up to 250 ms for
the detached recognition INSERT, and the provider settlement attribution write
is independently bounded to 250 ms and fail-open. A timeout or missing row is
retained as an explicit unavailable attribution gap. The append uses
`BEGIN IMMEDIATE` so concurrent physical completions cannot lose IDs; the
reader resolves the bounded list with one session-scoped SQLite join and
downgrades observed rows when IDs are malformed or ledger joins are missing.
Regression fixtures cover actual settlement-writer persistence, duplicate
replay rejection/no-effect, concurrent multi-call append, cross-session
rejection, overflow/malformed/unknown states, and a real close/reopen file DB
readback. The subsequent `recognition::tests` run executed 25 passing tests;
see the linked runtime receipt for its exact coverage and no-model boundary.
