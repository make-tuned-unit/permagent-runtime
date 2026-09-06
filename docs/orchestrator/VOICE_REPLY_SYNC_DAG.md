# V4 — Spoken reply synchronization audit and handoff

**Program:** `VOICE_UX_RELIABILITY_MASTER_PROGRAM_DAG.yaml`  
**Status:** component gate passed; device/accessibility acceptance owned by V6  
**Date:** 2026-09-04 (America/Halifax)  
**Production owner:** V3 Swift owner, after V2 observability receipt  
**Boundary:** this child is read-only. It does not modify `VoiceView.swift`,
`VoiceProtocolTypes.swift`, Rust, or the existing Spectral/session store.

## Audit verdict

**FAIL pending surgical fixes.** The current implementation has the right
overall shape: the daemon emits `audio_segment` metadata immediately before
PCM, uses UTF-16 offsets, the iOS client queues metadata in wire order, uses
the audio-player clock rather than arrival time in principle, renders one
scrollable `UITextView`, and clears state on stop. Existing tests prove basic
JSON decoding and a simple UTF-16 range.

It is not yet safe to claim Claude-like synchronized reading. The gaps are:

1. `VoiceReplyWordTiming` accepts negative, reversed, zero/invalid, and
   out-of-segment timings. The display range is validated later, but the time
   selection can still select a bad word or highlight a wrong range.
2. `displayRanges()` does not advance its fallback search cursor after an
   explicit range. Mixed explicit/fallback timings can therefore match the
   wrong repeated word.
3. The client pairs the next metadata frame with the next binary frame but
   does not reject duplicate/out-of-order segment IDs, missing PCM, unexpected
   sample rates, or an orphan binary frame. A malformed stream can desync all
   later highlights.
4. `reply_text` can replace/remap the durable reply after a segment has already
   entered `playbackItems`; those copied ranges are not remapped. Repeated
   segment text and server whitespace normalization can then highlight stale
   offsets.
5. The highlight task is driven by a wall-clock start timestamp. It does not
   compensate for audio-engine route changes, scheduling delay, interruption,
   or actual rendered sample time. A 50 ms poll is acceptable for display, but
   its source clock must be the audio player’s rendered timeline.
6. `scrollRangeToVisible` is called for every active word without tracking
   whether the user manually scrolled. This can wrest control from the reader;
   the contract requires the smallest reveal only when the active range leaves
   the viewport.
7. Fixed 240-point reply height is a reasonable compact baseline, but there is
   no tested adaptive conversation layout at 375 pt, Dynamic Type, or a
   three-paragraph reply while the orb and controls are present. The text is
   scrollable, but the visual acceptance gate is absent.
8. Missing timing metadata correctly remains readable, but the state is not
   explicitly distinguishable as “highlight unavailable”; this needs an
   internal/testable fallback state, not fabricated timing.
9. `voiceReplyWordIndex` returns the first word before its start and the last
   word after its end. That creates a highlight during leading/trailing silence
   and after the segment has finished. Gaps should be unhighlighted; a segment
   boundary should advance cleanly.
10. Accessibility exposes the whole reply, but no test verifies that highlight
    styling is supplemental, VoiceOver focus remains stable, and Reduce Motion
    prevents animated scroll jumps.

These are client synchronization and presentation issues, not a reason to
create another transcript/memory store. The authoritative reply remains the
existing voice turn/session path and the V2 privacy-safe telemetry remains the
diagnostic source.

## Existing contracts preserved

- V1 remains authoritative for `ready → listening → thinking → speaking →
  terminal`, user transcript visibility, full reply visibility, UTF-16 ranges,
  multi-paragraph behavior, accessibility, and no fabricated partials.
- V2 remains authoritative for capture/STT lifecycle and the 16:04 no-response
  diagnosis. V4 does not infer a successful reply from an audio frame.
- V5 remains authoritative for the actionable provider/model control and
  next-turn semantics. V4 must not change model selection or the Henry identity
  control.
- V3 owns the shared Swift production files. V4 supplies tests/acceptance
  criteria; it must not overwrite V3 transcript visibility work.

## Exact implementation handoff

### V4.1 — Normalize the timing protocol at the boundary

In the existing protocol helper, introduce a pure validation/normalization
step (no new store):

- require finite integer semantics already representable by `Int`;
- require `0 <= start_ms < end_ms <= duration_ms` when duration is positive;
- require UTF-16 ranges to be nonnegative and wholly within the segment’s
  `NSString` length;
- require the range text to be compatible with the supplied token, or fall
  back to forward token search;
- sort is not allowed to silently repair wire order: reject/mark the segment
  timing invalid if timings overlap or regress;
- retain audio and text when invalid, but set timing availability to
  `unavailable` so no highlight is produced.

The fallback search cursor must advance for both explicit and inferred ranges.
Repeated words must resolve left-to-right. Punctuation attached to a token is
allowed; whitespace is not part of the range.

### V4.2 — Make segment-to-audio pairing fail closed

Keep the existing one metadata → one PCM pairing, but add a small pure queue
state machine and receipt fields:

- segment IDs strictly increase within a reply;
- one binary frame consumes exactly one pending segment;
- an orphan binary, duplicate ID, missing metadata, invalid sample rate, or
  impossible duration clears timing for that segment/reply without dropping
  readable reply text;
- the queue is reset on `reply_start`, interruption, socket epoch change, and
  terminal reply state;
- no old-hub path may crash or fabricate a timing range.

Do not make this a server-side second protocol. The daemon’s existing ordered
metadata/PCM frame remains the source of truth.

### V4.3 — Use the rendered playback timeline

Keep the existing `AVAudioPlayerNode`, but derive elapsed playback from the
node’s rendered player time when available (`lastRenderTime` → `playerTime`),
with the current monotonic clock only as a documented fallback. Account for
the scheduled segment start and actual sample rate. On route/interruption or
player reset, stop the highlight task, clear the active word, and restart from
the new playback epoch rather than continuing stale time.

Do not increase polling frequency blindly. A 40–60 ms UI update is adequate;
correctness comes from the audio clock and deterministic boundary handling.

### V4.4 — Preserve reading control and scroll minimally

`VoiceReplyTranscriptView` must:

- scroll only when the active range is outside the visible text rect;
- avoid moving a user who manually scrolled if the active range remains
  visible;
- use no animated scroll under Reduce Motion;
- keep the current paragraph in view across `\n\n` boundaries;
- clear styling before applying the next valid range and clear it at every
  terminal/interruption path;
- keep the complete authoritative reply readable after audio ends.

The fixed height may remain initially, but V4.5 must prove it is usable at
375 pt with three paragraphs and Dynamic Type. If it is not, hand the smallest
layout change to V3 rather than rewriting the voice screen.

### V4.5 — Honest no-timing fallback and accessibility

No timing means readable audio/reply with no active-word styling. Expose this
only through internal state/tests and an optional nonintrusive accessibility
hint; never highlight proportional guesses on the client. The server’s
estimated timings remain valid when present, but malformed metadata is not
converted into a guess.

VoiceOver must read “Agent reply” and the full value independent of color or
bold styling. Dynamic Type, high contrast, and Reduce Motion must preserve
order and avoid focus theft.

## Deterministic verification fixtures

These fixtures are mandatory before V4 can pass. They are pure XCTest-level
tests and must not require a socket, paid provider, daemon build, or audio
device.

| ID | Fixture | Required result |
|---|---|---|
| SYNC-01 | `Hello world.` with valid UTF-16 ranges | exact ranges and active indices at start/middle/end |
| SYNC-02 | `I said “go 🚀 now.”` | emoji surrogate pair does not split a range; NSString length is used |
| SYNC-03 | `one one one` with inferred ranges | occurrences resolve left-to-right, not all to the first |
| SYNC-04 | `Wait—really? Yes!` | punctuation remains in token range and paragraph text remains unchanged |
| SYNC-05 | explicit first range followed by inferred repeated token | fallback begins after explicit range |
| SYNC-06 | negative, reversed, zero-length, overlapping, and out-of-bounds times | audio/text remain readable; no highlight and no crash |
| SYNC-07 | missing timings, empty timings, and malformed timing JSON | readable fallback; no fabricated active word |
| SYNC-08 | timings with a gap before/between/after words | no highlight in gaps or after segment end |
| SYNC-09 | three segments, `\n\n` boundaries, repeated text | global ranges map to the correct segment and paragraph |
| SYNC-10 | `reply_text` whitespace normalization after metadata and after PCM | active range is remapped or safely cleared; never stale |
| SYNC-11 | duplicate, regressed, missing, and orphan segment frames | queue fails closed and later turns recover |
| SYNC-12 | sample-rate mismatch and zero duration | timing unavailable; no divide-by-zero or accelerated highlight |
| SYNC-13 | interruption/route change/reset during speaking | highlight clears and new epoch starts cleanly |
| SYNC-14 | manual scroll with active word visible, then outside viewport | no jump while visible; smallest reveal when outside |
| SYNC-15 | 375 pt, three paragraphs, Dynamic Type, VoiceOver, Reduce Motion | complete text remains readable and controls remain reachable |

## Gates and receipt format

V4's component gate passes when all of the following are attached to the child
receipt:

1. source diff limited to the agreed Swift protocol/rendering files and test
   files; no Rust or memory-store changes;
2. `git diff --check` clean;
3. SYNC-01 through SYNC-13 pass in the project’s focused XCTest target;
4. a read-only source review confirms V3’s transcript partial/final behavior
   and V5’s model picker were not regressed;
5. SYNC-14 and SYNC-15, plus a fresh multi-paragraph device playback with at
   least three segment boundaries, remain explicit V6 program-acceptance gates.
   V4 may not substitute a stale app binary for that later evidence.

### Suggested receipt

```text
V4 status: passed | blocked
Changed files:
Focused tests: SYNC-01..SYNC-13 = 15/15 test methods
Timing invalidation: pass
Metadata/PCM pairing recovery: pass
Rendered-clock/route reset: pass
Manual-scroll/Reduce-Motion/375pt evidence: deferred to V6
V3/V5 regression review: pass
Fresh-build/device evidence: V6 receipt <id>
```

Until that receipt exists, the master DAG must keep `v4_reply_sync` active
and must not advance V6 solely because a basic decoder test passes.

## V4 implementation receipt (2026-09-04)

Implemented surgically in `VoiceProtocolTypes.swift`, `VoiceView.swift`, and
`VoiceIdleTests.swift`:

- malformed/reversed/zero/out-of-duration timing fails closed;
- explicit ranges advance inferred repeated-word search;
- gaps and leading/trailing silence have no active word;
- duplicate/regressed segment IDs disable timing for the reply while retaining
  readable audio/text;
- invalid sample-rate/timing segments remain readable without highlighting;
- durable `reply_text` remaps already queued playback ranges;
- rendered `AVAudioPlayerNode` time is preferred, with monotonic fallback;
- autoscroll only reveals an offscreen word and respects Reduce Motion;
- interruption/terminal reset clears highlight state;
- added pure `VoiceReplySegmentQueue` and global-range remap helpers so
  metadata/PCM framing, duplicate/regressed/orphan recovery, reset epochs,
  and repeated authoritative reply text are directly testable;
- pure fixtures now directly cover SYNC-01 through SYNC-13; SYNC-14/15 remain
  simulator/device visual and accessibility gates.

Verification:

- `git diff --check`: **passed**.
- `xcodebuild build` for `PermagentTests` against the generic iPhoneOS 26.5
  SDK with isolated derived data: **passed** (the full Swift test target
  compiled and linked).
- Focused XCTest execution on simulator
  `A82A2157-CFC5-41B9-91A7-611E5B75C7C9`: **passed**, `VoiceIdleTests` 15/15,
  0 failures (0.038 s test execution).
- The complete regenerated project suite is reported by the integrator as
  124/124 passed; this child directly verified the 15 V4 protocol tests.
- Fresh device multi-paragraph playback, route interruption, VoiceOver,
  Dynamic Type, and Reduce Motion evidence: **deferred to V6**.

V4's component gate is **passed**. Protocol gates SYNC-01–13 are passed;
SYNC-14/15 and fresh playback evidence are non-circular V6 exit gates. This
does not claim that the end-to-end voice program has passed.
