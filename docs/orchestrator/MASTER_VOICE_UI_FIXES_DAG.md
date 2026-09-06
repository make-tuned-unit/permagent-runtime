# Master Voice & UI Fixes DAG
**Date:** 2026-09-03  
**Scope:** Voice enrollment/tuning + kitchen-noise handling + voice-origin plumbing + UI fixes  
**Status:** Superseded as the historical kitchen/origin plan by the current
`MASTER_VOICE_UX_RELIABILITY_DAG_2026-09-04.md`; retain this document as the
prior voice-audit context and task history.

> Current pickup: the iOS design/no-response audit, Claude-informed component
> contract, stale-build gate, transcript visibility, reply highlighting, and
> model affordance work are organized in
> [`MASTER_VOICE_UX_RELIABILITY_DAG_2026-09-04.md`](MASTER_VOICE_UX_RELIABILITY_DAG_2026-09-04.md).

---

## Summary

Three converging problems from your voice-chat audit:
1. **Henry gets stuck on "Listening"** due to background noise (kitchen music, speakerphone hiss)
2. **Voice enrollment too strict** — tone variation across sentences fails enrollment
3. **Voice-origin plumbing incomplete** — Henry doesn't know which device is calling him

**This DAG sequences fixes across three parallel lanes:**
- **Spine (critical path):** Voice VAD tuning + noise rejection + enrollment
- **Origin lane:** Device identity + origin-aware prompts + tool policy  
- **Polish lane:** Markdown stripping, homographs, latency

**Verification lenses on every node:** completeness, wiring, regressions, latencies, bugs.

---

## The Three Root Problems & Solution Map

### Problem 1: Stuck on "Listening" (background noise)
**Root:** VAD does not distinguish kitchen music/hiss from speech. Uncommitted turns ride to 60s max instead of aborting on silence.

**Solution path (N0 → N1):**
- N0: Write failing tests (tripwires for all morning failures)
- N1: Port spectral veto from web + abort uncommitted noise on abortSilenceMs

### Problem 2: Voice Enrollment Fails (tone too variable)
**Root:** Current enrollment is sample-based, not speaker-embedding-based. Tone shift across sentences breaks the match.

**Solution path (N2 → N3):**
- N2: Enable Voice Isolation on iOS to clean the input signal
- N3: Replace sample enrollment with ONNX speaker-embedding model (like Siri does)

### Problem 3: Henry Doesn't Know Where He Is (no origin awareness)
**Root:** AuthPrincipal is discarded at the voice WS boundary. Henry defaults to "desktop" prompting even on iPhone.

**Solution path (Origin N1 → N2 → N4):**
- Origin N1: Thread device identity (ios_voice, watch_voice, desktop_voice) through VoiceReplyCtx
- Origin N2: Inject per-turn prompt: "you are on the iPhone, don't say 'on screen'"
- Origin N4: Gate navigate_app on non-phone clients; return no-op on ios_voice

---

## Master DAG Structure

```
SPINE (Voice Kitchen Fix)
  ├─ N0 Tripwire Tests ─────────┐
  │                              ├─ N1 VAD + Abort ─────┐
  ├─ N2 Voice Isolation ────────┤                      ├─ N4 Phone UI (show full reply)
  │                              ├─ N3 Speaker Embedding
  └─ (Polish: N6 Markdown, N8 Homographs, N9 Empty STT)

ORIGIN (Device Awareness)
  ├─ Origin N1 Device Contract ─┐
  │                              ├─ Origin N2 Prompt per device
  │                              ├─ Origin N3 Budget line per device
  └─ Origin N4 Tool Policy ──────┴─ Origin N5 Phone clipboard

DEPENDENCIES:
  N0 → N1 (tests gate VAD work)
  N1, N2 → N3 (enrollment needs clean signal + no noise)
  Origin N1 → Origin N2, Origin N3, Origin N4 (all need device identity)
  N3 + Origin N2 → N5 (show full reply on phone once we know it's the phone)
```

---

## Spine Lane: Voice Kitchen Fixes

### N0 — Tripwire Tests (No Production Changes)
**Goal:** Write FAILING tests that capture the 2026-08-27 morning failures.

**Files:**
- `ios/.../VoiceVADTests.swift` — speakerphone keepalive hiss >1s must abort, not max-turn
- `crates/goose/src/events/voice_pronounce.rs` — save_candidates must strip possessives
- `crates/goose-server/src/voice/proper_noun_corrector.rs` — "Pigkeeper's" must stay possessive
- `scripts/voice-latency-report.py` — parse 2026-08-27 sidecar logs

**Accept Criteria:**
- New tests FAIL on current main (no production fix yet)
- Four tripwires: empty 60s VAD, possessive strip, utterance-save, latency report parse
- CI red on these tests only; existing VAD tests still pass
- Baseline: first-audio median 4083 ms recorded in test comment

**Verification:**
- [ ] Completeness: one test per failure type
- [ ] Wiring: tests compile in PermagentMobileTests + goose unit targets
- [ ] Regressions: existing VoiceVADTests pass (natural pause, onset, barge-in)
- [ ] Latencies: N/A (no runtime change)
- [ ] Bugs: do not tune thresholds; only add failing tests

**Prompt:**
> Write N0 tripwire tests for the 2026-08-27 morning voice audit:
> 1. VoiceVADTests: speakerphone keepalive hiss at ~0.0055 RMS for >1s after uncommitted turn → abortSilenceMs, never ride to 60s maxTurnMs
> 2. voice_pronounce save_candidates: "You can't say \"pig keeper\"" must NOT store as sounds_like for pinkiepper
> 3. proper_noun_corrector: "What's the Pigkeeper's name?" → keep possessive, do not strip to Pigkeeper
> 4. voice-latency-report.py: parse the 2026-08-27T10:44 empty-STT snippet from ~/.permagent/logs/daemon-sidecar.log
> 
> Record baseline first-audio 4083ms. Do NOT fix production code; leave pronunciations.json alone.

---

### N1 — VAD Spectral Veto + Abort Uncommitted Noise
**Goal:** Port web spectral-veto (fail-open music detection) to iOS; abort uncommitted turns on silence.

**Why:** Kitchen music is broadband steady-state RMS. Current VAD treats it as voice onset. Speakerphone hiss rides 60s because there's no uncommitted-turn timeout. N1 makes Listening responsive: real speech opens a turn, music/hiss aborts in 500ms.

**Files:**
- `ios/.../VoiceVAD.swift` — abort uncommitted on abortSilenceMs (500ms); optional spectral veto hook
- `ios/.../VoiceView.swift MicPipe` — cheap FFT or band averages on tap (feed VAD)
- `ui/command-center/src/hooks/vadSpectrum.ts` — copy the fail-open contract (band AVERAGES, not dB sums)
- `ios/.../VoiceVADTests.swift` — N0 tripwire must go green

**Depends on:** N0 (tests must pass)

**Accept Criteria:**
- Hiss/music at cooking volume aborts past abortSilenceMs (500ms) + one onset streak
- A spoken "Henry, what's on my board?" still opens a turn
- N0 VAD test green
- Speakerphone, built-in, headset presets all abort uncommitted noise
- Hands-free default remains

**Verification:**
- [ ] Completeness: all three audio routes abort noise
- [ ] Wiring: FFT tap → vad.step; route change swaps presets without live-turn reset
- [ ] Regressions: testNaturalPausesDoNotEndTheTurn still passes (silenceMs 1400); onset streak still 2 frames; barge-in tests unchanged
- [ ] Latencies: abort ≤500ms not 60s; FFT tap must not drop frames (rms 0 streak)
- [ ] Bugs: do not re-enable setVoiceProcessingEnabled on speakerphone yet (that is N2)

**Prompt:**
> Implement N1: make speakerphone noise (hiss/music at kitchen volume) abort on abortSilenceMs instead of riding to 60s. Port spectrumLooksLikeVoice from web (band AVERAGES, not dB sums) to iOS mic tap so broadband/steady audio does not count as voicedAccumMs.
> 
> Keep silenceMs 1400ms for long turns. Abort must be ≤500ms. Turn N0 VAD tripwire green. Add synthetic "kitchen music" frame sequence in VoiceVADTests.
> 
> Do NOT enable setVoiceProcessingEnabled on speakerphone (that is N2). Do NOT add speaker embeddings (that is N3). Rebuild iOS to verify on device.

---

### N2 — Voice Isolation on Dictation & Speakerphone
**Goal:** Enable Apple Voice Isolation on iOS recording paths to clean enrollment + turn audio.

**Why:** Enrollment scores the waveform. If the waveform is Jesse + radio, the speaker embedding will be contaminated. Voice Isolation removes music/noise while you talk. DictateView, Notes, and chat composer need it. Speakerphone orb needs a measurement first (2026-08-21 AEC muted near-end mic on some devices).

**Files:**
- `ios/.../DictationRecorder.swift` — .spokenAudio + setVoiceProcessingEnabled(true)
- `ios/.../NotesView.swift NoteDictationRecorder` — same policy
- `ios/.../Views.swift chat composer` — uses DictationRecorder
- `ios/.../VoiceView.swift + VoiceAudioRoute.swift` — speakerphone isolation only if RMS still live
- `ios/.../VoiceAudioRouteTests.swift` — rewrite test with measured reason or revert

**Depends on:** N1 (noise already reduced by VAD)

**Accept Criteria:**
- Chat composer, Notes, DictateView record through Voice Isolation
- Speakerphone: either isolation ON + measured spoken syllable still exceeds onset, OR isolation OFF with measurement comment
- No 2026-08-21 "Listening, dead mic" regression
- AirPods VP stays on; MeetingCapture stays unprocessed (table-top room)

**Verification:**
- [ ] Completeness: chat/Notes/DictateView share one policy; watch path documented
- [ ] Wiring: setCategory/mode/setVoiceProcessingEnabled run before record; Control Center shows Isolation
- [ ] Regressions: external-output tests keep VP; MeetingCapture stays .default
- [ ] Latencies: STT upload ≤200ms vs today; first-audio median ≤4.1s
- [ ] Bugs: if speakerphone VP yields rms 0, revert in this node

**Prompt:**
> Implement N2: enable Voice Isolation on iOS one-way dictation (DictationRecorder + NoteDictationRecorder): AVAudioSession mode .spokenAudio + setVoiceProcessingEnabled(true). Shared policy so chat composer, Notes, and Notes-tab mic match.
> 
> For speakerphone orb: measure on a real iPhone whether setVoiceProcessingEnabled(true) still zeros near-end RMS (2026-08-21 bug). If it mutes, leave voiceProcessing false on speakerphone + write measurement in comment. If it does NOT mute, enable it and rewrite VoiceAudioRouteTests with that evidence.
> 
> AirPods keep VP on. MeetingCapture stays unprocessed. Do NOT build embeddings (that is N3). Rebuild/reinstall iOS app to verify dictation.

---

### N3 — Speaker Embedding Enrollment (ONNX Model)
**Goal:** Replace sample-based enrollment with speaker-embedding model so tone variation doesn't break re-enrollment.

**Why:** Current enrollment: "record 3 sentences → compare waveforms on later turns". Tone shift across sentences breaks the match. Speaker embeddings (like Siri): "extract voice print → compare embeddings on later turns". Embeddings are invariant to tone, speed, and pitch. Use ONNX (same substrate as STT/TTS).

**Files:**
- `crates/goose-server/src/voice/speaker_print.rs` (new) — ONNX model load + embedding extraction
- `crates/goose-server/src/routes/voice.rs` — enroll start/stop frames; score embedding on commit before STT
- `ios/.../VoiceView.swift` — enrollment mode UI (3 orb prompts, reuse teachWord chrome)
- `~/.permagent/data/voice_print.json` — embedding + created_at (never raw WAV)
- Tests: fail-open when no print exists; score logging (no raw audio)

**Depends on:** N2 (clean audio for enrollment) + N1 (no noise in captured buffer)

**Accept Criteria:**
- Settings / first-run: say 3 sentences → Henry confirms enrollment
- With a print: kitchen radio / another speaker does not produce transcript/reply
- Your voice over radio still transcribes (N2 helps)
- No print: behavior identical to today
- Score logged; no raw audio in logs

**Verification:**
- [ ] Completeness: enroll, re-enroll, delete, skip all work; watch/desktop document fail-open or hub-print sharing
- [ ] Wiring: enrollment mode on iOS; model download like kws.rs; score before STT in voice.rs
- [ ] Regressions: fresh install (no print) still hears you; existing STT flow unchanged
- [ ] Latencies: embedding extraction ≤100ms; no added first-audio delay
- [ ] Bugs: do not store raw WAV; fail OPEN when no print

**Prompt:**
> Implement N3: speaker-embedding enrollment using ONNX. Like Siri, extract a voice print (embedding) from 3 enrollment sentences, score all incoming audio before STT. If similarity < threshold, send idle (no reply). If no print, fail open (today's behavior).
> 
> Files: speaker_print.rs (ONNX + embedding), voice.rs (enroll frames + score before STT), iOS VoiceView (enrollment UI reusing teachWord chrome).
> 
> Storage: ~/.permagent/data/voice_print.json with embedding + created_at, never raw audio. Log similarity/admit/reject without raw audio. Watch/desktop: document fail-open or hub-print shared enrollment.
> 
> N3 allows tone variation — unlike sample waveforms, embeddings are invariant to pitch/speed/tone shift. Requires N2 clean audio + N1 no noise.

---

### N4 — Phone UI: Show Full Reply Text
**Goal:** Make iOS VoiceView reply text scrollable instead of clipped to 6 lines.

**Why:** "The rest is on screen" doesn't apply to iPhone. Full reply must be readable on the orb screen. N3's speaker embedding means a 2-minute spoken turn is possible; current 6-line clip is untenable.

**Files:**
- `ios/.../VoiceView.swift conversationText` — remove lineLimit(6); make scrollable
- Optional: Continue control that sends next spoken turn

**Depends on:** N3 (longer turns now possible) + Origin N2 (Henry knows it's the phone)

**Accept Criteria:**
- Full reply_text scrollable on orb screen
- 6-line clip removed
- Optional Continue control

**Verification:**
- [ ] Completeness: scrollable + no clip
- [ ] Wiring: replyText binding still live; no orb rebuild needed
- [ ] Regressions: layout still centered, touch scroll intuitive
- [ ] Latencies: N/A
- [ ] Bugs: do not break orb animation

**Prompt:**
> Implement N4: iOS VoiceView conversationText is currently lineLimit(6), clipping the reply. Remove the clip and make the full reply_text scrollable on the orb screen. Do not rebuild the orb. Optional: a Continue control that sends the next spoken turn. Rebuild/reinstall iOS app to verify on device.

---

### N6 — Strip Markdown in TTS (Sidecar)
**Goal:** Remove markdown markers before sending to Kokoro TTS.

**Why:** You heard "**Angle one:**" spoken aloud. speakable() drops UUIDs but not markdown.

**Files:**
- `crates/goose-server/src/voice/speakable.rs` — strip `**`, `#`, `-`, `>` before TTS

**Accept Criteria:**
- `speakable("**Angle one:** People…")` returns prose without asterisks
- Kokoro log has no markdown

**Prompt:**
> Implement N6: strip markdown markers in speakable.rs (**bold**, headings, bullets) before Kokoro TTS. Independent of origin plumbing.

---

### N8 — Homograph Lexicon (Sidecar)
**Goal:** Store homographs (live vs live, read vs read) so STT disambiguation is learned.

**Why:** "You said the word is live, not live." STT collapsed both to one spelling. Henry had no phonetic contrast.

**Files:**
- `crates/goose-server/src/voice/user_lexicon.rs` — add homograph path
- `crates/goose-server/src/voice/pronunciation_coaching.rs` — teach homographs
- Tests: homograph stored and used on later turns

**Accept Criteria:**
- Taught homograph (live = LYVE vs LIV) is stored
- Used on next turn
- Unresolved: he asks which one instead of guessing

**Prompt:**
> Implement N8: add homograph path to user_lexicon.rs + pronunciation_coaching.rs so "live vs live" can be taught and stored. Unresolved homographs prompt for clarification instead of guessing. Independent of origin.

---

### N9 — Empty STT Handling (Sidecar)
**Goal:** Don't surface false "No speech detected" errors when VAD aborts noise.

**Why:** Three recordings this call came back transcript empty (barge-in / VAD reject). Current path toasts "No speech detected" as a failed turn.

**Files:**
- `ios/.../VoiceVAD.swift` — distinguish VAD abort from empty transcript
- `crates/goose-server/src/routes/voice.rs` — empty transcript branch (silent reconnect vs error)

**Accept Criteria:**
- Reconnect or short noise does not toast error
- Real speech still transcribes
- VAD-abort flows silently back to Listening

**Prompt:**
> Implement N9: distinguish VAD abort (empty by design) from empty STT (no transcript despite valid audio). Silent reconnect on VAD abort; only toast error if real audio came back empty. Requires N1 VAD changes.

---

## Origin Lane: Device Awareness

### Origin N1 — Keep Device Identity (AuthPrincipal Thread)
**Goal:** Preserve AuthPrincipal through voice WS + thread device name into VoiceReplyCtx.

**Why:** Auth middleware already knows "iPhone" from the certificate. Voice route discards it immediately. Until device identity survives, every prompt assumes desktop.

**Files:**
- `crates/goose-server/src/middleware/auth.rs` — return AuthPrincipal from voice WS path
- `crates/goose-server/src/routes/voice.rs` — VoiceQuery + VoiceReplyCtx thread device
- `crates/goose-server/src/device_registry.rs` — device name for device_id
- `ios/.../VoiceView.swift` — client=ios_voice on WS query
- `ui/command-center/src/hooks/useVoice.ts` — client=desktop_voice

**Accept Criteria:**
- Daemon log on iPhone connect: session_id, device name "iPhone", client ios_voice
- Desktop logs desktop_voice
- AuthPrincipal not discarded

**Verification:**
- [ ] Completeness: iOS + desktop + watch all thread client
- [ ] Wiring: VoiceQuery has client param; VoiceReplyCtx carries device name; auth.rs returns principal
- [ ] Regressions: non-voice auth flows unchanged
- [ ] Latencies: N/A (metadata only)
- [ ] Bugs: principal must survive entire turn

**Prompt:**
> Implement Origin N1: keep AuthPrincipal on /voice WebSocket and thread device identity into VoiceReplyCtx. Add client query param (ios_voice | watch_voice | desktop_voice). Resolve device name from registry. Log origin on connect. Wire iOS VoiceView + desktop useVoice.ts. Do NOT change prompts yet (that is Origin N2).

---

### Origin N2 — Per-Device Prompt Injection
**Goal:** Inject device context into system prompt every turn.

**Why:** After Origin N1, Henry knows where the call comes from. Tell him the implications: phone cannot see Command Center or hub browser; speak answers; never say "on screen".

**Files:**
- `crates/goose-server/src/routes/voice.rs extend_system_prompt` — voice_origin + voice_reply_style variants

**Accept Criteria:**
- On ios_voice: "you are on the iPhone, they cannot see Command Center or hub browser, talk the answer, do not say on screen, do not navigate_app unless they ask to change this phone"
- On desktop_voice: remainder may live in transcript

**Verification:**
- [ ] Completeness: ios_voice, watch_voice, desktop_voice all have variants
- [ ] Wiring: VoiceReplyCtx.client → extend_system_prompt
- [ ] Regressions: non-voice flows unchanged
- [ ] Latencies: N/A (text only)
- [ ] Bugs: do not gate tools yet (that is Origin N4)

**Prompt:**
> Implement Origin N2: inject per-turn voice_origin system prompt from VoiceReplyCtx. Phone/watch: cannot see desktop surfaces; speak the answer; never say on screen. Desktop: remainder may live in transcript. Do NOT gate tools yet (that is Origin N4).

---

### Origin N3 — Origin-Aware Budget Line
**Goal:** Vary the spoken-budget notice by device.

**Why:** N0 stops "on screen" damage. N3 makes the cue match the device: phone/watch offer to keep talking; desktop can point to transcript.

**Files:**
- `crates/goose-server/src/routes/voice.rs push_spoken, BUDGET_NOTICE variants`

**Accept Criteria:**
- ios_voice: "continue by voice"
- desktop_voice: "rest is in the transcript"
- No client still uses old generic line

**Verification:**
- [ ] Completeness: both ios and desktop have variants
- [ ] Wiring: VoiceReplyCtx.client → push_spoken
- [ ] Regressions: budget flow unchanged
- [ ] Latencies: N/A
- [ ] Bugs: N/A

**Prompt:**
> Implement Origin N3: make spoken-budget notice depend on VoiceReplyCtx.client. Phone/watch: offer to continue speaking. Desktop: remainder is in the transcript. Tests for both.

---

### Origin N4 — Tool Policy: No Desktop Navigation on Phone
**Goal:** Gate navigate_app on non-phone; return explicit no-op on ios_voice.

**Why:** Henry opened reckonize.org in hub browser while you were on iPhone. iOS already ignores navigate frames, but tool still succeeds, so he talks as if the tab switched. Set policy: phone cannot drive the desktop.

**Files:**
- `crates/goose/src/agents/platform_extensions/app_conductor.rs navigate_app`
- `crates/goose/src/app_catalog.rs to_prompt_block`
- `crates/goose-server/src/routes/voice.rs` — origin in tool-facing prompt

**Accept Criteria:**
- ios_voice: navigate_app returns explicit no-op ("this client cannot switch Command Center")
- He doesn't claim a tab opened
- Browser read tools still allowed (he narrates what he found, never "look at the page")

**Verification:**
- [ ] Completeness: navigate_app gated; browser reads still allowed
- [ ] Wiring: VoiceReplyCtx.client → tool-facing prompt
- [ ] Regressions: desktop voice unchanged; non-voice flows unchanged
- [ ] Latencies: N/A
- [ ] Bugs: tool result is honest

**Prompt:**
> Implement Origin N4: on ios_voice/watch_voice, navigate_app must not claim desktop tab switch. Return explicit no-op result. Keep browser tools for reading, but origin prompt (Origin N2) already forbids implying the user can see hub browser. Tests for the no-op.

---

### Origin N5 — Clipboard on the Speaking Device
**Goal:** Fix copy_to_clipboard so it lands on the device doing the voice call, not the hub.

**Why:** 12:08 voice turn: Henry said "it's on the clipboard" but Notes paste was empty. Intercept should copy to iPhone pasteboard, not desktop.

**Files:**
- `crates/goose/src/events/clipboard_intercept.rs`
- `crates/goose-server/src/routes/voice.rs` — Clipboard frames
- `ios/.../VoiceView.swift` — UIPasteboard on clipboard frame

**Accept Criteria:**
- From iPhone voice: copy_to_clipboard lands on iPhone pasteboard
- Paste into Notes works
- Tool result honest if client never ack'd

**Verification:**
- [ ] Completeness: ios + desktop both work
- [ ] Wiring: clipboard_intercept → WS frame → iOS UIPasteboard
- [ ] Regressions: non-voice clipboard unchanged
- [ ] Latencies: N/A
- [ ] Bugs: no swallowed success

**Prompt:**
> Implement Origin N5: fix copy_to_clipboard for voice so it lands on the device making the voice call. Trace clipboard_intercept → WS clipboard frame → iOS UIPasteboard. Verify Notes paste works. Tool result is honest about device-specific success.

---

## UI Fixes from Codex Session

### AF1 — Spacing Consolidation (In Progress)
**Status:** Recent commits show spacing sweep onto space scale (b0ebaa68, bac1b1cd).

**Accept Criteria:**
- Voice picker / pronunciation spacing on space scale
- Build view / cost status line spacing on space scale
- Native title= calls replaced with Tooltip primitive

**Verification:**
- Run UI type checks + visual tests

---

## Execution Order & Dependencies

**Phase 1 (Parallel, Weeks 1–2):**
- N0: Write tripwire tests (no production changes)
- Origin N1: Thread device identity through WS

**Phase 2 (Parallel, Weeks 2–3):**
- N1: Port spectral VAD + abort uncommitted (green on N0)
- Origin N2 + N3: Prompt injection + budget variants (depend on Origin N1)
- N2: Enable Voice Isolation on dictation

**Phase 3 (Parallel, Weeks 3–4):**
- N3: Speaker embedding enrollment (depends on N1 + N2)
- Origin N4 + N5: Tool gating + clipboard routing (depend on Origin N1)

**Phase 4 (Weeks 4+):**
- N4: Phone UI scrollable reply (depends on N3 + Origin N2)
- N6, N8, N9: Polish (independent)

---

## Verification Checklist

- [ ] All N0 tripwire tests FAIL on main before starting spine work
- [ ] N1 VAD green + N0 tripwires turn green
- [ ] N2 Voice Isolation: no 2026-08-21 regression (test on real device)
- [ ] N3 enrollment: tone variation now works (re-enroll with different tone)
- [ ] Origin N1 logs device identity in daemon log
- [ ] Origin N2 prompt varies by device (grep daemon log for ios_voice vs desktop_voice context)
- [ ] Origin N4 navigate_app returns no-op on ios_voice (test result)
- [ ] N4 phone UI scrollable (rebuild iOS, verify on device)
- [ ] All tests pass; no regressions in non-voice flows

---

## Notes for Pickup

1. **Codex prepared:** voice-origin-dag.canvas.tsx (origin plumbing 9 nodes) + ios-voice-kitchen-dag.canvas.tsx (kitchen noise 7 nodes)
2. **Cursor project:** /Users/j/.cursor/projects/Users-j-Documents-dev-permagent-runtime/canvases/
3. **Branch strategy:** Use goal/* branches for each worker; review & land to main only after Decision Inbox approval
4. **Model escalation:** If a node blocks (e.g., device measurement for N2), escalate to reviewer or stronger model
5. **Cost optimization:** Delegate mechanical tasks (markup strip, latency report parse) to cheap subagent; keep hard reasoning (DSP tuning, embeddings) on main loop

---

## Success Criteria (End State)

✅ **Henry no longer gets stuck on "Listening"** (N1 VAD + abort)  
✅ **Voice enrollment works with tone variation** (N3 speaker embedding)  
✅ **Henry knows where he is** (Origin N1–N5 device routing)  
✅ **Phone shows full reply, clipboard works** (N4, Origin N5)  
✅ **All regressions blocked** (tests on all nodes)
