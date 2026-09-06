# Voice Chat Audit Summary
**Date:** 2026-09-03  
**Session:** Henry voice chat with background noise & enrollment issues  
**Status:** Three root problems identified; structured DAG created for fixes

---

## Audit Findings

### Problem 1: Henry Gets Stuck on "Listening" (Background Noise)
**Symptom:** Kitchen music and speakerphone hiss cause 60-second "Listening" hangs instead of aborting.

**Root Cause:**
- Current VAD does not distinguish music/hiss from speech
- Uncommitted turns have no timeout; they ride to 60s maxTurnMs
- Speakerphone keepalive (~0.0055 RMS steady) accumulates voicedAccumMs and never clears

**Impact:** Six 60-second empty listens in the morning session.

**Solution:** 
- Port spectral veto from web (fail-open music detection) to iOS VAD
- Add abortSilenceMs (500ms) for uncommitted turns
- Abort broadband steady-state (music/hiss) before it hits onset threshold

**Task:** TASK-N1 (VAD Spectral Veto + Abort Uncommitted)

---

### Problem 2: Voice Enrollment Too Strict (Tone Variation Fails)
**Symptom:** Re-enrollment fails when your tone changes across sentences ("sentence 1" high, "sentence 2" low, "sentence 3" mid). Current system matches waveforms sample-by-sample.

**Root Cause:**
- Enrollment is sample-based (direct audio comparison)
- Tone, pitch, speed variations look like different speakers
- No speaker-embedding model (like Siri uses)

**Impact:** Cannot re-enroll; stuck with original print or no enrollment.

**Solution:**
- Replace sample-based enrollment with ONNX speaker-embedding model
- Extract voice print from 3 enrollment sentences (embedding is tone/speed/pitch invariant)
- Score incoming audio via embedding similarity (not waveform shape)
- Fail open if no enrollment exists (today's behavior)

**Prerequisites:**
- N2: Enable Voice Isolation on iOS so enrollment audio is clean (no music/background noise contaminating the embedding)
- N1: VAD already rejected pure noise, so committed audio is clean

**Task:** TASK-N3 (Speaker Embedding Enrollment)

---

### Problem 3: Henry Doesn't Know Where He Is (No Origin Awareness)
**Symptom:** Henry assumes desktop environment; says "on screen", "navigate tab", suggests looking at Command Center even when you're on iPhone. He's unaware of the speaking device.

**Root Cause:**
- AuthPrincipal (device identity) is extracted at middleware but discarded at voice WS boundary
- VoiceQuery + VoiceReplyCtx have no device context
- System prompt assumes desktop always
- navigate_app succeeds on phone (tab doesn't switch, but he claims it did)
- copy_to_clipboard lands on hub desktop, not iPhone pasteboard

**Impact:** Nonsensical suggestions; unactionable instructions; clipboard doesn't work on phone.

**Solution:**
- Origin N1: Thread AuthPrincipal + device name through voice WS into VoiceReplyCtx
- Origin N2: Inject per-device prompt ("you are on iPhone, speak answers, don't say on screen")
- Origin N3: Vary budget notice by device ("continue by voice" vs. "rest in transcript")
- Origin N4: Gate navigate_app on non-desktop clients; return no-op on ios_voice
- Origin N5: Route copy_to_clipboard to the speaking device's pasteboard
- N4: Make iOS reply scrollable (now that we know it's the phone)

**Tasks:** TASK-Origin-N1 through TASK-Origin-N5, plus TASK-N4 (Phone UI)

---

## Structured Fix Plan

**Three parallel lanes** (see MASTER_VOICE_UI_FIXES_DAG.md for detailed node specs):

### Spine Lane (Voice Kitchen Fix)
```
N0 Tripwire Tests (no code changes, just tests)
 ↓
N1 VAD Spectral Veto + Abort Uncommitted
 ├─ N2 Voice Isolation (dictation cleanup)
 │   ↓
 │  N3 Speaker Embedding Enrollment
 │   ↓
 │  N4 Phone UI – Scrollable Reply
 │
 └─ N6 Strip Markdown in TTS (sidecar)
 └─ N8 Homograph Lexicon (sidecar)
 └─ N9 Empty STT Handling (sidecar)
```

### Origin Lane (Device Awareness)
```
Origin N1 Device Contract (thread identity)
 ├─ Origin N2 Per-Device Prompt Injection
 ├─ Origin N3 Origin-Aware Budget Line
 ├─ Origin N4 Tool Policy (no navigate_app on phone)
 └─ Origin N5 Clipboard on Speaking Device
```

---

## Success Criteria (End State)

✅ **Henry no longer gets stuck on "Listening"** (N1 VAD + abort)  
✅ **Voice enrollment works across tone variations** (N3 speaker embedding)  
✅ **Henry knows which device is calling** (Origin N1 device identity)  
✅ **Henry knows what device CAN'T do** (Origin N2–N4 prompt + tool policy)  
✅ **Phone shows full reply scrollable** (N4 UI)  
✅ **Clipboard works from iPhone** (Origin N5)  
✅ **All regressions blocked** (tests on every node)

---

## Implementation Phases

| Phase | Duration | Workers | Tasks | Status |
|-------|----------|---------|-------|--------|
| Phase 1 | Wk 1–2 | Codex, Claude Code | N0 tripwires, Origin-N1 routing | Ready to start |
| Phase 2 | Wk 2–3 | Codex, Claude Code | N1 VAD, Origin N2/N3, N2 isolation | Depends on Ph1 |
| Phase 3 | Wk 3–4 | Codex, Claude Code | N3 enrollment, Origin N4/N5 | Depends on Ph2 |
| Phase 4 | Wk 4+ | Cheap subagent | N4 UI, N6/N8/N9 polish | Independent |

---

## Files Created for Pickup

1. **MASTER_VOICE_UI_FIXES_DAG.md** (505 lines)
   - Full DAG spec with dependencies, verification lenses, prompts
   - All 13 nodes (Spine + Origin + Polish)

2. **VOICE_FIXES_TASK_BREAKDOWN.md** (522 lines)
   - Delegable tasks for workers
   - Each task: scope, deliverables, verification, done criteria
   - Phase 1–4 sequencing

3. **VOICE_AUDIT_SUMMARY.md** (this file)
   - Audit findings, root causes, solution map
   - Quick reference for the three problems

---

## Next Steps

1. **Confirm with user:** Review the three root causes + solution map
2. **Assign Phase 1 to workers:**
   - Codex: TASK-N0 (tripwire tests) + TASK-N1 (VAD) by end of week
   - Claude Code: TASK-Origin-N1 (device routing) by end of week
3. **Create goal branches:** For each task (goal/voice-n0-tripwire-tests, etc.)
4. **Set Decision Inbox gate:** All Phase 1 tasks require review approval before landing to main
5. **Integration testing:** Once Phase 2 is done, test on real iPhone with kitchen noise + re-enroll at different tones

---

## Key Constraints & Assumptions

- **iOS device required:** Must test N1–N4 on real iPhone (XCTest + on-device)
- **N2 measurement:** Speakerphone Voice Isolation test requires real device (RMS check on iOS 18+)
- **ONNX model source:** Speaker embedding model needs licensing review (sherpa-onnx or ONNX Runtime + trained model)
- **Voice Isolation availability:** iOS 16+ (fallback for older iOS: skip N2 or warn user)
- **FFT overhead:** MicPipe tap with spectrum must not drop frames; monitor CPU on device during N1 integration
- **No breaking changes:** Non-voice flows must stay identical; all new code is voice-specific or origin-specific

---

## Risk Mitigation

| Risk | Mitigation |
|------|-----------|
| Speakerphone RMS zeros on N2 | Measure on real device before enabling; keep VP off if it mutes |
| Embedding model licensing | Verify ONNX model license upfront (sherpa-onnx is Apache 2.0) |
| FFT tap CPU overhead | Profile on target devices; fall back to band averages if needed |
| VAD false negatives (missing speech) | Fail-open: if unsure, lean toward "speech"; existing barge-in tests guard |
| Device routing regressions | All non-voice auth flows tested; only voice WS path changes |
| Clipboard frame loss | Log every clipboard event with device origin; sync on reconnect |

---

## Questions for Follow-Up

1. **ONNX Model:** Which speaker-embedding model do you prefer? Options:
   - sherpa-onnx (Apache 2.0, open)
   - TensorFlow Lite (Google, needs conversion)
   - PyTorch (custom, needs export)

2. **Voice Isolation:** iOS version requirement? (16+, 17+, 18+?)

3. **Speaker Embedding Threshold:** Similarity score for admit/reject? (suggest 0.8, tunable)

4. **Re-enrollment UX:** In Settings: "Re-enroll voice" button, or auto-offer if score dips?

5. **Integration Testing:** Access to real iPhone with kitchen environment to verify N1 + N2 + N3 together?

---

## Audit Notes (Raw)

**Session time:** ~90 minutes (estimated from symptoms)  
**Device:** iPhone (speakerphone mode, kitchen environment)  
**Issues logged:**
- 06:00–07:00: Six 60-second "Listening" hangs (background music/hiss)
- 07:01–07:45: Re-enrollment attempted 3 times, failed (tone variation)
- 08:00–08:45: Tone-drift mid-conversation caused dropout
- 12:08: Henry said "it's on the clipboard" but Notes paste empty (clipboard not on iPhone)
- Navigation attempts: "open reckonize.org in hub browser" while on iPhone → he claimed tab opened (but user never saw it)

**Baseline audio quality:** First-audio median 4083ms (record in N0 tests for regression detection)

---

## References

- Codex session: voice-origin-dag.canvas.tsx (9 nodes, stored in Cursor projects)
- Codex session: ios-voice-kitchen-dag.canvas.tsx (7 nodes, stored in Cursor projects)
- Prior work: fix/voice-pronounce-drill (PR #1159)
- Prior work: fix/aac-channel-detection (branch exists)
- Settings: Voice Isolation control in iOS 16+ (UISwitch in AVAudioSession)
- TTS: Kokoro already integrated; speakable() pipeline (crates/goose-server/src/voice/speakable.rs)

