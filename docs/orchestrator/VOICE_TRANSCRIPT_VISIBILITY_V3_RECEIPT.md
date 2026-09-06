# V3 receipt — user transcript visibility and conversation-first voice UI

Date: 2026-09-04  
Scope: `ios/PermagentMobile/PermagentMobile/VoiceView.swift`,
`VoiceProtocolTypes.swift`, and focused voice tests.

## Implemented

- Final user STT is presented as a distinct `YOU` card with readable 17-point
  text; provisional STT remains visibly provisional (`Transcribing…`) and is
  replaced by the final transcript through the existing `VoiceTranscriptBuffer`.
- The conversation moves above the orb once user text or an agent reply exists.
  The shared particle orb remains the idle primary affordance, then contracts
  from 300 to 144 points so a 375–430 point phone does not push the words below
  the fold.
- Existing reply word highlighting and UIKit auto-scroll remain the playback
  clock; no live partial producer is claimed because the current daemon is
  batch STT.
- Empty captures, malformed/short captures, transport loss, and voice errors
  retain a bounded, recoverable terminal message until the next turn. Legacy
  `idle`/`error` frames remain supported. The additive `turn_outcome` frame is
  decoded when available.
- No Spectral/session storage, provider routing, agent identity affordance, or
  daemon route was changed.

## Verification gates

- Pure state gate: `VoiceTurnFeedback` covers empty capture, connection loss,
  error fallback, recoverability, and clear-on-next-turn semantics in
  `VoiceIdleTests`.
- Wire compatibility gate: `turn_outcome` is optional and falls back to the
  existing `transcript`, `error`, and `idle` handling for older daemons.
- Layout gate: conversation text is rendered before the compact orb whenever
  a transcript/reply/status exists; no fixed 300-point orb is retained in that
  state.
- Passed: Swift parse and `git diff --check` after the final additive
  `turn_outcome` handling.
- Passed before that final switch: full iOS test-target build and 5/5 focused
  `VoiceIdleTests`. A second identical simulator invocation failed in
  CoreSimulator/DerivedData services before assertions ran; this is recorded as
  `infrastructure_failed`, not a source/test assertion failure. V6 owns the
  fresh clean build and device rerun.
- Pending device gate: V6 must rebuild the daemon/app and verify a real turn;
  this change does not claim to repair a stale installed binary or prove live
  partial STT.
