# Master Voice UX + Reliability Program DAG

**Date:** 2026-09-04 (America/Halifax)
**Status:** V0–V5 passed; V6 active and blocked at the fresh device boundary
**Owner:** Permagent orchestrator
**Program manifest:** `VOICE_UX_RELIABILITY_MASTER_PROGRAM_DAG.yaml`
**Portfolio controller:** `PERMAGENT_IMPROVEMENT_PORTFOLIO_DAG.yaml`

## Objective

Bring the iOS voice and chat surfaces to a Claude-like conversation model while
preserving Permagent's existing hub WebSocket, Spectral/session storage, and
voice pipeline:

1. Make the user's speech visible as soon as the protocol has a partial or final
   transcript.
2. Keep the complete agent reply visible, highlight the word currently being
   spoken, and scroll the active word through multi-paragraph replies.
3. Make the chat identity/model affordance tappable and obvious. Provider →
   model selection must be expanded, deterministic, and applied on the next
   turn without restarting the current turn.
4. Diagnose and eliminate silent/no-response turns, with a terminal outcome and
   useful latency evidence for every capture.
5. Verify the app and daemon are rebuilt from the same source before attributing
   a result to a fix.

This is a sequence of child DAGs. Each child has a bounded implementation and
verification loop, an explicit write scope, and a clean handoff artifact. The
master may run independent read-only evidence collection in parallel, but no
two children may edit the same production file at once.

## Evidence lock: what the screenshots and logs actually prove

### Screenshot evidence

The Claude screenshots (Images 1–9 in the request) are **reference evidence**:
they show a visible conversation transcript, a completed voice-chat card, and a
provider/model picker whose choices are expanded and selectable. They do not
prove any particular Claude implementation detail or establish a requirement
that Permagent copy its visual assets.

The current Permagent screenshots are **product evidence**:

| Evidence | Observed fact | Source corroboration | Status |
|---|---|---|---|
| Image 10 | Voice surface shows `VOICE`, `Henry`, a CPU icon, an orb, `LISTENING FOR YOU`, and a hands-free control; no user transcript is visible | `VoiceView.swift:1510-1553`, `1568-1595`, `1735-1741` | Proven UI state |
| Image 11 | Chat composer shows an inert `Henry` capsule between `+` and voice controls | `Views.swift:942-947` is a plain `Text`, not a `Button` | Proven implementation gap |
| Images 1–9 | Claude's composer/model surface exposes a model affordance and voice history/transcript | Visual reference only | Proven reference, not a code specification |

### 16:04 AST voice incident

The authoritative server logs are:
`/Users/j/.permagent/logs/daemon-sidecar.log` and its rotated server copy
`/Users/j/.permagent/logs/server/2026-09-03/20260903_195445-permagentd.log`.
At 19:04:20–19:05:02Z (16:04:20–16:05:02 AST), session `20260904_6`:

- a session was created (`POST /api/sessions`, 200);
- an iOS voice socket connected (`client=ios_voice`, `device=iPhone`);
- three recordings stopped at approximately 1.8s, 1.6s, and 9.3s;
- all three synchronous STT results were empty (`23ms`, `23ms`, `70ms`);
- there was no transcript, provider request, TTFT, TTS, reply, or persisted message.

Spectral's existing session database corroborates that session
`20260904_6` has zero messages. This is evidence of a no-response user
experience, but not evidence of the precise lower-level cause (for example,
bad PCM, a VAD decision, an STT model failure, or a stale executable).

The same daemon-sidecar log gives an adjacent failure signal from 19:00–19:02Z:
11 captures produced 6 audible responses while 5 had empty STT. The audible
turns had first-audio latency median 2,024.5ms, p90 5,949.5ms, maximum 8,234ms;
TTFT was median 1,287.5ms, p90 3,673.5ms, maximum 4,641ms. Every audible turn
was subsequently classified as interrupted, with socket reconnects commonly
within one second of first audio. This proves reply truncation/socket churn in
that window. It does **not** prove the user caused the interruptions: playback
bleed, false barge-in, client reconnect policy, and intentional taps remain
hypotheses. V2/V6 must add client timing and playback/mic correlation before
changing barge-in policy.

The running desktop daemon/app were built before the current source changes.
The daemon executable mtime was 2026-09-03 13:01 AST while `routes/voice.rs`
was modified 2026-09-04 08:05 AST. Therefore the 16:04 result cannot be used
as a validation of the current source until the rebuild child DAG passes.

No named `com.permagent.mobile` unified-log records were available for this
window. Do not invent client-side timings or infer that the phone transmitted
zero audio. The server did receive enough samples to report 1.6–9.3 seconds of
capture, but the logs do not include a sample-level RMS/peak or PCM validity
record for this incident.

### Current source facts relevant to the audit

- `VoiceView.swift:1096-1104` handles `transcript_partial` and final
  `transcript` frames, but `routes/voice.rs:1418-1425` explicitly documents
  that the daemon currently performs batch STT only after `Stop`. The iOS
  partial-render path is therefore wired defensively but cannot display live
  words until a streaming STT producer exists.
- `VoiceView.swift:60-124` already uses a scrollable UIKit text view and active
  range styling; `VoiceView.swift:1107-1114` pairs audio metadata with text;
  `VoiceView.swift:1307-1339` drives highlighting from playback time. This is
  promising existing work, not yet a verified device behavior.
- `VoiceProtocolTypes.swift:25-169` accepts multiple timing field spellings
  and derives UTF-16 display ranges. It needs order, malformed-timing, and
  multi-paragraph tests before the feature is called complete.
- `ModelPickerView.swift:67-109` has an expandable provider → model list and
  `ModelPickerView.swift:151-207` applies voice/session scope on the next
  selection. The current CPU icon and chat identity capsule are separate UX
  gaps; do not rewrite the working provider endpoint contract.

The sidecar currently labels stages only on the daemon side. The next telemetry
pass must join at minimum `session_id`, `turn_id`, socket epoch, playback
start/end, reconnect reason, and client state transitions. A server-side
`interrupted` label alone cannot identify a user-originated barge-in.

## Master dependency graph

```text
V0 Evidence lock (read-only)
 ├── V1 Visual/component contract ─────────────┐
 ├── V2 Capture/STT observability + diagnosis ──┼── V6 Rebuild + device E2E
 ├── V3 User transcript visibility ─────────────┤       │
 ├── V4 Reply timing/highlight/autoscroll ──────┤       ▼
 └── V5 Identity/model affordance ──────────────┘── V7 Master acceptance
                                                   │
                                                   ▼
                                           Handoff to next master DAG
```

V1–V5 may perform independent audits in parallel. Production implementation
of V2 must precede V3's live-transcript claim. V3 and V4 may be implemented in
parallel only if their shared protocol edits are serialized by the orchestrator
and the second child re-validates the first child's receipt. V6 is a hard gate:
no screenshot or voice claim is accepted from an unreconstructed app/daemon.

## Child DAG contracts

Each node below is itself a DAG with `entry`, `work`, `verification`, `exit`,
and `retry` gates. A child that cannot satisfy its entry gate must emit
`blocked` with evidence; it must not silently broaden scope.

### V0 — Evidence lock and stale-build audit

**Write scope:** none (read-only; report under `docs/orchestrator/`).

**Entry gate:** preserve the current worktree; identify the source revision,
running app/daemon PIDs, executable mtimes, target session, and available logs.

**Work:** reconcile screenshot claims with source lines, inspect the 16:04 AST
window, inspect the existing Spectral session row, and classify each statement
as `observed`, `source-confirmed`, `hypothesis`, or `unavailable`.

**Verification gate:** the evidence table includes a path and line/time range
for every asserted behavior; no claim relies on a simulator as a real-mic test;
no new memory, ledger, or scheduler is introduced.

**Exit receipt:** this file's Evidence Lock section plus a machine-readable list
of the exact session/log paths. **Current status: passed.**

**Retry bound:** one re-read after any process restart; otherwise stop and report
the missing evidence.

### V1 — Claude-informed visual/component contract

**Write scope:** documentation and visual fixtures only during audit. Later UI
implementation may edit `VoiceView.swift`, `Views.swift`, `ModelPickerView.swift`,
and their tests, but only after V1 exits.

**Entry gate:** V0 passed; product reference screenshots are available; no
assumption is made that the reference's internals should be copied.

**Work DAG:**

1. Map voice states (`connecting`, `ready/listening`, `listening`, `thinking`,
   `speaking`, `failed`) to visible transcript, status, affordances, and
   recovery actions.
2. Decide the chat capsule contract. Recommended contract: show the active
   model/provider in the Claude-like composer position and make it a `Button`
   opening the existing provider → model picker; keep Henry as the agent identity
   in the header/accessibility copy. If product intent is instead agent switching,
   document the exact agent endpoint before implementation.
3. Choose a model-switch icon that communicates configuration/model selection,
   not hardware (`cpu`); validate at 1x, 2x, Dynamic Type, and Reduce Motion.
4. Define the placement and minimum height of user transcript, reply transcript,
   active word, and failure/retry notice so the orb never hides the only feedback.

**Verification gate:** a reviewable state/interaction matrix and reference
screenshots/fixtures for iPhone portrait widths 375–430pt; Voice, Chat, and
Control surfaces agree on provider/model terminology; the Henry capsule has no
inert state.

**Exit receipt:** `VOICE_UI_COMPONENT_CONTRACT.md` (child artifact) with
approved copy, accessibility labels, and exact component owners.

**Retry bound:** two visual-review iterations; unresolved ambiguity is a
`needs_decision` receipt, not an implementation guess.

### V2 — Capture → STT observability and no-response diagnosis

**Write scope:** first audit is log/test fixtures only. Implementation may edit
`crates/goose-server/src/routes/voice.rs`, `scripts/voice-latency-report.py`,
and voice test fixtures; it must not edit iOS UI files in the same child.

**Entry gate:** V0 passed; running binary freshness is recorded; no provider
request or paid test call is allowed without the existing task budget gate.

**Work DAG:**

1. Add/reuse privacy-safe per-turn stages: socket ready, capture started,
   first frame received, frame count/bytes, sample rate, bounded finite-sample
   health (count/RMS/peak only), capture stopped, STT started/completed,
   transcript outcome (`nonempty`, `empty`, `error`), provider started, first
   audio, terminal outcome. Never log PCM or transcript content in telemetry.
2. Make every turn terminal: `capture_rejected_short`, `empty_stt`, `stt_error`,
   `speaker_rejected`, `provider_error`, `reply_sent`, or `disconnected`.
3. Extend the existing latency report to distinguish “no agent invoked” from
   “agent invoked but no audio returned”; preserve current voice log formats.
4. Replay the 16:04 session fixture and a known-good session fixture. Do not
   call the result a device fix until V6 uses a rebuilt daemon.
5. Add a reply-completion invariant: queued audio is not complete until the
   final segment drains, and each reconnect records whether it came from
   client/user barge-in, route recovery, or transport failure. This guards the
   19:00–19:02 truncation pattern.

**Verification gate:** deterministic fixtures cover no frames, finite silence,
NaN/Inf input, valid speech-like PCM, empty STT, STT error, provider failure,
and socket close. Each path produces exactly one terminal outcome and no
provider call when STT is empty. Existing latency-report tests pass.

**Exit receipt:** `VOICE_CAPTURE_STT_AUDIT.md` plus fixture output identifying
the exact missing stage for any reproduced no-response turn.

**Retry bound:** two focused test-fix cycles; no endless “verification” loop.

### V3 — User transcript visibility (partial + final)

**Write scope:** `ios/PermagentMobile/PermagentMobile/VoiceView.swift`,
`VoiceProtocolTypes.swift`, and iOS voice transcript tests; if a daemon producer
is required, hand it to V2 rather than editing `routes/voice.rs` opportunistically.

**Entry gate:** V1 contract passed; V2 defines the capture/STT event contract.

**Work DAG:**

1. Verify the existing final transcript binding with mocked `transcript` frames.
2. Add a protocol-backed streaming-STT child only if the daemon can provide
   safe, ordered partials. Partial frames must be marked provisional and may be
   replaced by exactly one final transcript.
3. Keep partial text visible during capture/decoding; on empty STT show an
   honest recoverable status rather than leaving the user staring at
   `LISTENING FOR YOU` with no explanation.
4. Preserve UTF-16 correctness, accessibility value “What you said,” and turn
   reset semantics.

**Verification gate:** mock sequence `ready → start → partial(1) → partial(2) →
final → reply` renders each stage; `final` supersedes partial without flicker;
empty/error/disconnect states render a terminal recovery action; tests cover
very long, emoji, punctuation, and multi-line input. If streaming STT is not yet
available, the exit receipt explicitly says “final-only, producer pending,”
not “live transcript complete.”

**Exit receipt:** `VOICE_TRANSCRIPT_VISIBILITY_DAG.md` and passing XCTest
evidence.

**Retry bound:** two focused cycles; protocol mismatch returns to V2.

### V4 — Spoken reply transcript, word highlight, and autoscroll

**Write scope:** `VoiceView.swift`, `VoiceProtocolTypes.swift`, and reply
rendering tests only. The daemon timing producer is V2's handoff.

**Entry gate:** V1 layout contract passed; current timing wire shape recorded;
no visual acceptance from the stale binary.

**Work DAG:**

1. Test audio metadata and binary-frame pairing in arrival order.
2. Test timing ranges with ASCII, emoji, apostrophes, repeated words,
   punctuation, missing ranges, zero/negative/out-of-order times, and a
   `reply_text` frame arriving before or after segment metadata.
3. Keep the entire reply in a bounded, user-scrollable transcript; while the
   agent is speaking, scroll the active word into view and style only that word.
   Do not jump to the bottom merely because a long reply arrived.
4. On missing timings, keep audio/reply readable and suppress invalid highlight;
   on interruption/reconnect, cancel the old highlight timeline and clear stale
   ranges.

**Verification gate:** a multi-paragraph fixture proves every spoken segment
   receives the correct active-word range and is brought into view; no crash or
   invalid range occurs; manual scroll is not corrupted outside active playback;
   accessibility exposes full reply text. Existing `VoiceOrbDriveTests` and
   `VoiceIdleTests` remain green.

**Exit receipt:** `VOICE_REPLY_SYNC_DAG.md` with simulator screenshots showing
   at least three paragraphs and highlight progression.

**Retry bound:** two focused cycles; malformed wire data is fixed at the parser
   boundary rather than patched with UI special cases.

### V5 — Functional identity/model affordance and icon pass

**Write scope:** `ios/PermagentMobile/PermagentMobile/Views.swift`,
`VoiceView.swift`, `ModelPickerView.swift`, and associated iOS tests. Do not
change provider configuration endpoints without a separate daemon contract.

**Entry gate:** V1's capsule contract resolved; existing `/config`,
`/config/providers`, `/config/set_provider`, and `/config/model-route` behavior
is captured in tests.

**Work DAG:**

1. Replace the inert chat `Text(identity.nameCapitalized)` capsule with the
   approved active provider/model action opening the existing model picker.
   Preserve Henry as identity in the Voice header, but make that identity an
   explicit button opening the existing typed agent controls/roster; never
   overload it with model selection.
2. Keep provider rows expandable, show configured models only, preserve custom
   current model fallback, and apply selection to the next turn.
3. Replace the CPU glyph with the V1-approved configuration/model icon; update
   accessibility labels and Voice/Control surfaces consistently.
4. Add loading, unavailable, refusal, and retry states without silently changing
   the active model.

**Verification gate:** UI tests tap the composer/header affordance and observe
   the expanded provider list; tap a model and verify the correct scoped POST;
   verify current-turn model stability and next-turn switch; verify keyboard,
   VoiceOver, Dynamic Type, and narrow-width layouts. No control is visually
   tappable while functionally inert.

**Exit receipt:** `VOICE_MODEL_AFFORDANCE_DAG.md` plus screenshots and endpoint
   mock transcript.

**Retry bound:** two focused cycles; endpoint/schema failures hand back to the
   API owner rather than adding client-side guesses.

### V6 — Rebuild, daemon/app freshness, and device E2E gate

**Write scope:** build outputs and evidence artifacts only; no source edits.

**Entry gate:** V2–V5 receipts exist, focused tests pass, and all current
   source changes are visible in the intended build revision.

**Work DAG:**

1. Rebuild/restart the daemon from current `routes/voice.rs` and confirm PID,
   executable mtime, and source/build revision.
2. Regenerate/build/install the iOS target from the same worktree revision.
3. Run deterministic simulator UI tests for visual layout and interaction.
4. Run a real-device matrix: clean short utterance, long multi-paragraph
   response, barge-in, no-speech/background noise, provider/model switch, and
   socket interruption. Simulator audio is not a substitute for this matrix.
5. Collect client and daemon privacy-safe timestamps and correlate by session/
   turn ID. Record failures as terminal outcomes, not “still verifying.”

**Verification gate:** binary freshness proves both sides include the intended
   changes; every matrix case has a screenshot/log receipt; no-response cases
   identify a terminal stage; first-audio and endpoint metrics are reported
   separately; no raw audio or secrets are stored.

**Exit receipt:** `VOICE_REBUILD_DEVICE_E2E_DAG.md` with build IDs, device/OS,
   test matrix, screenshots, and latency percentiles.

**Retry bound:** one rebuild retry after a clean process stop; one device retry
   per failed case. Repeated failure is `blocked` with logs, not a loop.

### V7 — Master acceptance and handoff

**Write scope:** master evidence and status only.

**Entry gate:** V6 passed; no child is `active`, `unknown`, or `needs_decision`.

**Verification gate:**

- Claude-inspired affordance is functionally equivalent in intent, not copied in
  implementation or branding.
- User transcript is visible whenever a producer emits partial/final text, and
  final-only behavior is honestly labeled until streaming STT exists.
- Agent reply remains readable across multiple paragraphs; active spoken word
  highlight/autoscroll is correct or safely suppressed when timing is absent.
- Henry/model affordance is tappable and its action is explicit.
- The 16:04 failure is either reproduced with a precise terminal cause or
  re-tested successfully against freshly rebuilt binaries; stale evidence is not
  counted as a pass.
- All focused tests, source diff checks, and app/daemon build gates pass.
- A known-good long reply completes without unexplained socket churn; any
  interruption has a correlated client/daemon cause and is not attributed to
  the user's voice without playback/mic evidence.

**Exit receipt:** update this document's status and hand the next executable
node to the parent master program. If a physical iPhone is unavailable, V7 must
remain `blocked_on_device_evidence`; it must not be marked excellent from
simulator screenshots alone.

## Cost, routing, and verification policy

- Assign read-only log extraction, fixture generation, and visual comparison to
  the least expensive configured worker that can satisfy the contract.
- Assign protocol/daemon changes to a Rust-capable worker familiar with the
  existing voice route; assign Swift/UI changes to a Swift-capable worker.
- A reviewer receives the child receipt and changed-file manifest before any
  next child starts. The reviewer may reject a broad rewrite or an unverified
  claim, but may not reopen passed gates without new evidence.
- Each child gets at most two implementation/verification retries (V6 has the
  explicit rebuild/device bounds above). A failed test is actionable evidence;
  repeating the same command without a changed input is not verification.
- All persistence remains in the existing Spectral/session/voice-log paths.
  This program creates no parallel memory, ledger, scheduler, or transcript
  store.

## Current frontier

V0–V5 are passed. **V4** passed its component boundary: SYNC-01 through
SYNC-13 are covered by 15 focused tests, and the full regenerated test target
previously passed 124/124 on the concrete iPhone 17 Pro simulator. SYNC-14 and
SYNC-15 are intentionally owned by **V6**, because manual scroll, Dynamic Type,
VoiceOver, Reduce Motion, real playback, and microphone behavior cannot be
proved by pure protocol tests.

**V6** is active but blocked at the freshness/device boundary. Its latest
bounded receipt records a daemon typecheck and test-bundle build pass, a stale
daemon executable after a disk-exhausted build, an application-scheme WatchKit
configuration failure, unavailable CoreSimulator, and no connected physical
device. V6 must remain active until a materially changed environment supports
a fresh daemon/app build and the real-device microphone/audio/accessibility
matrix. The same unchanged failed commands must not be looped.

## V3b — Incremental STT producer gap

September 5 continuation audit supersedes the older V6 environment note:
simulator tests and signed phone installation now pass (150 tests); the phone
remains locked for live acceptance. More importantly, the daemon still uses
final-only batch STT while iOS accepts `transcript_partial`. The V3 XCTest
receipt proves the consumer, not live producer availability. Rebuilding alone
does not close that gap.

The master now explicitly activates V3b before V6, preserving prior consumer
test evidence. Audit the configured STT transport first; implement only a
supported bounded partial path using existing capture/turn ownership. A
partial must be provisional, scoped to the current generation, replaced by
the authoritative final, and suppressed after cancellation/disconnect. No
unbounded re-transcription loop, raw audio logging, new cloud service or
unapproved paid partial calls. Verify partial/final ordering, empty results,
timeouts, stale generations and interruption. Then rerun V6 on fresh artifacts.
