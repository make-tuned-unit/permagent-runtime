# Voice Fixes Task Breakdown
**For Delegation to Workers**

Structured breakdown of the Master DAG into discrete, delegable tasks. Each task is scoped to a single worker goal branch.

---

## Phase 1: Tripwires & Device Routing (Weeks 1–2)

### TASK-N0: Tripwire Tests
**Worker:** Codex or Claude Code  
**Branch:** `goal/voice-n0-tripwire-tests`  
**Scope:** Write FAILING tests only; no production changes  
**Estimated effort:** 4–6 hours

**Deliverables:**
1. `ios/PermagentMobile/PermagentMobileTests/VoiceVADTests.swift`
   - Test: speakerphone keepalive hiss (RMS ~0.0055) >1s after uncommitted turn → abortSilenceMs, never max-turnMs 60s
   - Should FAIL on current main

2. `crates/goose/src/events/voice_pronounce.rs` → test
   - Test: save_candidates("You can't say the word \"pig keeper\"") must NOT store as sounds_like for pinkiepper
   - Should FAIL on current main

3. `crates/goose-server/src/voice/proper_noun_corrector.rs` → test
   - Test: "What's the Pigkeeper's name?" keeps possessive 's, does not strip to Pigkeeper
   - Should FAIL on current main

4. `scripts/voice-latency-report.py`
   - Parse 2026-08-27 snippet from ~/.permagent/logs/daemon-sidecar.log
   - Classify empty-STT turn as incomplete
   - Should FAIL on current main

5. Test file comments record:
   - Baseline first-audio median: 4083ms

**Verification:** `cargo test -p permagent --lib voice_pronounce` + `cargo test -p goose-server --lib proper_noun_corrector` + XCTest VoiceVADTests + pytest scripts/voice-latency-report.py → all FAIL

**Done when:** All four tests written, CI red on these tripwires only, existing VAD tests still pass.

---

### TASK-Origin-N1: Device Identity Contract
**Worker:** Claude Code  
**Branch:** `goal/voice-origin-n1-device-contract`  
**Scope:** Thread AuthPrincipal + device name through voice WS  
**Estimated effort:** 6–8 hours

**Deliverables:**

1. `crates/goose-server/src/middleware/auth.rs`
   - Extract AuthPrincipal on voice WS path (currently discarded)
   - Return principal to voice.rs

2. `crates/goose-server/src/routes/voice.rs`
   - Add `client: String` field to VoiceQuery (values: ios_voice, watch_voice, desktop_voice)
   - Add `device_name: String`, `client: String` to VoiceReplyCtx
   - Parse client param from WS query
   - Resolve device_name from device_registry

3. `crates/goose-server/src/device_registry.rs`
   - Add method: `fn device_name(&self, device_id: &str) -> String`

4. `ios/PermagentMobile/PermagentMobile/VoiceView.swift`
   - On voice WS connect, include query param: `client=ios_voice`

5. `ui/command-center/src/hooks/useVoice.ts`
   - On voice WS connect, include query param: `client=desktop_voice`

6. Logging
   - On voice WS connect, log: `device_name={name}, client={client}, session_id={id}`

**Verification:** 
- Daemon log on iOS app voice start: "device_name=iPhone, client=ios_voice, session_id=..."
- Daemon log on desktop voice start: "device_name=..., client=desktop_voice, session_id=..."
- AuthPrincipal survives entire voice turn (check VoiceReplyCtx in all voice handlers)

**Done when:** 
- [ ] iOS logs ios_voice
- [ ] Desktop logs desktop_voice
- [ ] AuthPrincipal not discarded
- [ ] All voice tests pass

---

## Phase 2: Noise Rejection & Prompt Injection (Weeks 2–3)

### TASK-N1: VAD Spectral Veto + Abort Uncommitted
**Worker:** Codex  
**Branch:** `goal/voice-n1-vad-spectral-veto`  
**Scope:** Port web spectral veto; abort uncommitted noise on silence  
**Estimated effort:** 10–14 hours  
**Depends on:** TASK-N0 (tripwire tests)

**Deliverables:**

1. `ios/PermagentMobile/PermagentMobile/VoiceVAD.swift`
   - Add `abortSilenceMs: Int = 500` parameter
   - Logic: after audio input starts but BEFORE user has committed to a turn (no speech detected), if silence lasts >abortSilenceMs, abort the turn (return to idle/Listening)
   - Add optional spectral veto hook (or inline)

2. `ios/PermagentMobile/PermagentMobile/VoiceView.swift MicPipe`
   - Tap raw audio → compute RMS + spectrum (cheap FFT or band averages)
   - Feed spectrum to vad.step()
   - Must not drop frames (avoid rms 0 streak)

3. Spectral veto logic (port from web)
   - `ui/command-center/src/hooks/vadSpectrum.ts` — study the fail-open contract (band AVERAGES, not dB sums)
   - On iOS: broadband/steady kitchen audio (music, hiss) → lower voicedAccumMs or signal "not-voice"
   - Fail open: if unsure, lean toward "speech"; never block real speech

4. `ios/PermagentMobile/PermagentMobileTests/VoiceVADTests.swift`
   - Add test: speakerphone keepalive hiss for >1s → abortSilenceMs ≤500ms (tripwire N0 should now PASS)
   - Add test: synthetic "kitchen music" frame sequence → abort
   - Add test: real speech "Henry, what's on my board?" → turn opens (still passes)

**Verification:**
- [ ] N0 tripwire test PASSES on VoiceVADTests
- [ ] Kitchen music >1s at cooking volume → abort in ≤500ms
- [ ] Hiss at speakerphone→ abort in ≤500ms
- [ ] Real speech still opens turn
- [ ] testNaturalPausesDoNotEndTheTurn still passes (silenceMs 1400)
- [ ] Onset streak still requires 2 frames
- [ ] Barge-in tests unchanged
- [ ] FFT tap does not drop frames
- [ ] All three audio routes (speakerphone, built-in, headset) abort noise

**Done when:**
- [ ] iOS app rebuilt; on-device test: hiss in kitchen ~60dB SPL holds Listening <1s, not 60s
- [ ] Real speech ("Henry…") still opens turn
- [ ] XCTest VoiceVADTests all pass
- [ ] TASK-N0 tripwires all green

---

### TASK-Origin-N2+N3: Device-Aware Prompts & Budget
**Worker:** Claude Code  
**Branch:** `goal/voice-origin-n2-n3-prompts`  
**Scope:** Inject device context + vary budget line  
**Estimated effort:** 6–8 hours  
**Depends on:** TASK-Origin-N1 (device identity in VoiceReplyCtx)

**Deliverables:**

1. `crates/goose-server/src/routes/voice.rs` → `extend_system_prompt()`
   - Read `VoiceReplyCtx.client`
   - For ios_voice:
     ```
     "You are speaking on the iPhone. The user cannot see the Command Center 
      or the hub browser. Always speak your answer. Do not mention what's on 
      the screen or tell them to look at anything. Never navigate to a different 
      app unless the user specifically asks you to. Keep answers concise and 
      spoken."
     ```
   - For watch_voice:
     ```
     "You are speaking on the Apple Watch. The user cannot see the Command Center 
      or hub browser. Always speak your answer. Do not mention the screen. Keep 
      answers very short."
     ```
   - For desktop_voice:
     ```
     "The user can see the Command Center and hub browser. You may reference what's 
      visible on screen or navigate. The remainder of your answer can be written."
     ```

2. `crates/goose-server/src/routes/voice.rs` → `push_spoken()` — update BUDGET_NOTICE variants
   - For ios_voice: "Say continue and I'll keep going."
   - For watch_voice: "Say continue."
   - For desktop_voice: "The rest is in the transcript."

3. Tests
   - Test extend_system_prompt(ios_voice) contains "speak your answer"
   - Test extend_system_prompt(desktop_voice) contains "written"
   - Test push_spoken(ios_voice) contains "continue"
   - Test push_spoken(desktop_voice) contains "transcript"

**Verification:**
- [ ] Daemon log on ios_voice turn: system prompt includes "speak your answer"
- [ ] Daemon log on desktop_voice turn: system prompt includes "written"
- [ ] ios_voice budget line: "Say continue and I'll keep going."
- [ ] desktop_voice budget line: "The rest is in the transcript."
- [ ] Non-voice flows unchanged

**Done when:**
- [ ] All tests pass
- [ ] Grep daemon log for ios_voice/desktop_voice turns → prompts are device-correct
- [ ] Budget lines are spoken correctly

---

### TASK-N2: Voice Isolation on Dictation
**Worker:** Codex  
**Branch:** `goal/voice-n2-voice-isolation`  
**Scope:** Enable Apple Voice Isolation on iOS recording  
**Estimated effort:** 8–12 hours  
**Depends on:** TASK-N1 (noise reduction from VAD)

**Deliverables:**

1. `ios/PermagentMobile/PermagentMobile/DictationRecorder.swift` (or equivalent)
   - Set AVAudioSession category to .playAndRecord (or .spokenAudio if available)
   - Set mode to .spokenAudio
   - Call setVoiceProcessingEnabled(true) before record start
   - Consistent policy for chat, Notes, and standalone dictation

2. `ios/PermagentMobile/PermagentMobile/NotesView.swift NoteDictationRecorder`
   - Same voice isolation policy as DictationRecorder
   - Shared RecorderPolicy struct or base class

3. `ios/PermagentMobile/PermagentMobile/Views.swift chat composer`
   - Uses DictationRecorder → inherits voice isolation

4. `ios/PermagentMobile/PermagentMobile/VoiceView.swift + VoiceAudioRoute.swift`
   - **Measure on real iPhone:** does setVoiceProcessingEnabled(true) on speakerphone zero RMS (2026-08-21 bug)?
   - If RMS zeros: leave voiceProcessing false on speakerphone, document measurement in code comment
   - If RMS is live: enable it, rewrite VoiceAudioRouteTests with measured evidence
   - AirPods: keep VP on
   - MeetingCapture: leave unprocessed (table-top room recording)

5. `ios/PermagentMobile/PermagentMobileTests/VoiceAudioRouteTests.swift`
   - Update/rewrite tests to reflect measured behavior (either "VP off on speakerphone" or "VP on + measured RMS live")

**Verification:**
- [ ] Chat dictation: STT never hears kitchen music in background
- [ ] Notes dictation: Voice Isolation active (check Control Center)
- [ ] Speakerphone orb: either VP off with measurement comment, or VP on with evidence
- [ ] No "Listening, dead mic" regression from 2026-08-21
- [ ] AirPods VP unchanged (on)
- [ ] Latency: STT upload ≤200ms vs today, first-audio median ≤4.1s

**Done when:**
- [ ] iOS app rebuilt & reinstalled
- [ ] On device: dictate through chat → Control Center shows "Voice Isolation" active
- [ ] Speakerphone: either documented off (with measurement) or on (with evidence)
- [ ] XCTest VoiceAudioRouteTests pass
- [ ] No regressions in other audio routes

---

## Phase 3: Enrollment & Tool Gating (Weeks 3–4)

### TASK-N3: Speaker Embedding Enrollment (ONNX)
**Worker:** Codex  
**Branch:** `goal/voice-n3-speaker-embedding`  
**Scope:** Replace sample-based enrollment with ONNX speaker embeddings  
**Estimated effort:** 14–18 hours  
**Depends on:** TASK-N1 (clean VAD) + TASK-N2 (Voice Isolation)

**Deliverables:**

1. `crates/goose-server/src/voice/speaker_print.rs` (new)
   - Load ONNX model (use same infrastructure as kws.rs for model download)
   - Function: `extract_embedding(&audio_bytes: &[u8]) -> Result<Vec<f32>>`
   - Function: `score_similarity(&embedding1: &[f32], &embedding2: &[f32]) -> f32` (cosine similarity)
   - Fail-open: if no model, return similarity 1.0 (admit all)

2. `crates/goose-server/src/routes/voice.rs`
   - On voice commit (end of turn audio):
     - Extract embedding from committed audio
     - Compare to stored enrollment (from ~/.permagent/data/voice_print.json)
     - If similarity >= threshold (e.g., 0.8): proceed to STT
     - If similarity < threshold: send idle (no reply, no error toast)
     - If no enrollment stored: fail open (admit all)
   - Log similarity score (no raw audio in logs)

3. Enrollment flow (iOS)
   - `ios/PermagentMobile/PermagentMobile/VoiceView.swift`
   - Add enrollment mode UI (reuse teachWord chrome)
   - Prompt: "Say 3 different sentences to enroll your voice."
   - On enroll complete: send "enroll_complete" frame with audio
   - Daemon receives, extracts embedding, saves to ~/.permagent/data/voice_print.json

4. Settings UI
   - Add "Re-enroll voice", "Delete voice print" controls
   - Display last-enrolled date

5. Storage
   - `~/.permagent/data/voice_print.json`:
     ```json
     {
       "embedding": [0.123, -0.456, ...],
       "created_at": "2026-09-03T20:00:00Z",
       "updated_at": "2026-09-03T20:00:00Z"
     }
     ```
   - Never store raw audio

6. Tests
   - Test: enrollment extract + score works
   - Test: same speaker re-enroll with different tone → similarity >= 0.8
   - Test: different speaker → similarity < 0.8
   - Test: no enrollment → fail open (admit all)
   - Test: score logged without raw audio

**Verification:**
- [ ] On iOS: run enrollment 3 times with varying tone → all succeed
- [ ] Daemon log: similarity scores logged (e.g., "similarity=0.87, admit")
- [ ] Daemon log: no raw audio in logs
- [ ] With enrollment: another speaker's voice → similarity < 0.8 → idle (no reply)
- [ ] With enrollment: kitchen radio over your speech → (N2 Voice Isolation helps; N1 VAD rejects pure music)
- [ ] No enrollment: behavior identical to today (fail open)
- [ ] Latency: embedding extraction ≤100ms
- [ ] All tests pass

**Done when:**
- [ ] iOS app rebuilt; on-device: enroll 3x with different tones → success
- [ ] Verify a second speaker → no reply (silent idle)
- [ ] Verify your voice over background → N2 isolation + N1 VAD help
- [ ] Unit tests pass; embedding score logged without audio
- [ ] Settings show "Last enrolled: [date]"

---

### TASK-Origin-N4+N5: Tool Gating & Clipboard
**Worker:** Claude Code  
**Branch:** `goal/voice-origin-n4-n5-tools`  
**Scope:** Gate navigate_app on phone; fix clipboard routing  
**Estimated effort:** 8–10 hours  
**Depends on:** TASK-Origin-N1 (device identity in VoiceReplyCtx)

**Deliverables:**

1. `crates/goose/src/agents/platform_extensions/app_conductor.rs` → navigate_app
   - Check if running in voice context with ios_voice or watch_voice client
   - If phone/watch: return explicit no-op result:
     ```
     {
       "status": "no_op",
       "reason": "This device cannot switch apps in the hub. You're speaking on an iPhone/Watch."
     }
     ```
   - If desktop_voice or non-voice: proceed normally

2. `crates/goose/src/app_catalog.rs` → to_prompt_block
   - When building tool context for ios_voice:
     - Remove navigate_app from available tools OR
     - Include in tool list but with note: "navigate_app unavailable on this device"
   - Browser read tools (read_webpage, search) still available

3. `crates/goose-server/src/routes/voice.rs` — add to tool-facing context:
   - "The user is on an iPhone/Watch. They cannot see the hub browser. Do not tell them to navigate or look for something on screen. Describe what you found, do not imply they can see it."

4. `crates/goose/src/events/clipboard_intercept.rs`
   - On clipboard event from voice context:
     - Preserve device origin (which device is doing the voice call)
     - Route clipboard write to that device, not the hub

5. `crates/goose-server/src/routes/voice.rs` — clipboard frame handling
   - Detect copy_to_clipboard frame from agent
   - Route to the speaking device's pasteboard (via device connection)

6. `ios/PermagentMobile/PermagentMobile/VoiceView.swift`
   - Implement UIPasteboard write on clipboard WS frame
   - Verify paste into Notes works

7. Tests
   - Test: navigate_app on ios_voice → no-op result with reason
   - Test: navigate_app on desktop_voice → normal result
   - Test: copy_to_clipboard on ios_voice → logged with device origin
   - Test: browser tools still available on all clients

**Verification:**
- [ ] ios_voice turn calls navigate_app → result: "no_op, this device cannot switch apps"
- [ ] He does not claim a tab opened
- [ ] Desktop voice navigate_app still works normally
- [ ] iOS copy_to_clipboard → iPhone pasteboard → Notes paste works
- [ ] Daemon log shows device origin on clipboard event
- [ ] Browser read tools still work on phone (he narrates findings, never says "look")

**Done when:**
- [ ] All tests pass
- [ ] iOS app rebuilt; voice to iPhone: copy emoji → paste in Notes succeeds
- [ ] Desktop voice: navigate_app still works
- [ ] No tool availability regressions on non-voice flows

---

## Phase 4: Polish & Phone UI (Weeks 4+)

### TASK-N4: Phone UI — Scrollable Reply
**Worker:** Claude Code  
**Branch:** `goal/voice-n4-phone-ui`  
**Scope:** Make iOS VoiceView reply text scrollable  
**Estimated effort:** 4–6 hours  
**Depends on:** TASK-N3 (longer turns now possible) + TASK-Origin-N2 (Henry knows it's the phone)

**Deliverables:**

1. `ios/PermagentMobile/PermagentMobile/VoiceView.swift`
   - Find: `conversationText` property with `lineLimit(6)`
   - Remove lineLimit; make scrollable (ScrollView or similar)
   - Full reply_text displayed on orb screen

2. Optional: Continue control
   - Add a "Continue" button/control in the transcript
   - Tapping it sends another voice turn (reuses last transcript context)

3. No orb rebuild required

**Verification:**
- [ ] Full reply scrollable on device
- [ ] Orb layout still centered
- [ ] Touch scroll intuitive
- [ ] Orb animation unchanged

**Done when:**
- [ ] iOS app rebuilt & reinstalled
- [ ] On device: a 2-minute reply is fully readable (scrollable, not clipped)
- [ ] Layout still looks polished

---

### TASK-N6: Markdown Stripping in TTS
**Worker:** Cheap subagent  
**Branch:** `goal/voice-n6-markdown-strip`  
**Scope:** Remove markdown before Kokoro TTS  
**Estimated effort:** 2–3 hours

**Deliverables:**

1. `crates/goose-server/src/voice/speakable.rs`
   - Function: `strip_markdown(text: &str) -> String`
   - Remove: `**bold**` → bold, `#heading` → heading, `-bullet` → bullet, `>quote` → quote
   - Tests: "**Angle one:** People…" → "Angle one: People…" (no asterisks in log or TTS)

**Verification:**
- [ ] `speakable("**Angle one:**…")` returns no asterisks
- [ ] Kokoro log clean

**Done when:**
- [ ] Tests pass
- [ ] Kokoro log has no markdown

---

### TASK-N8: Homograph Lexicon
**Worker:** Cheap subagent  
**Branch:** `goal/voice-n8-homographs`  
**Scope:** Store homograph pairs (live vs live)  
**Estimated effort:** 4–6 hours

**Deliverables:**

1. `crates/goose-server/src/voice/user_lexicon.rs`
   - Add: `fn add_homograph(&mut self, word: &str, pronunciations: Vec<String>)`
   - Storage: `~/.permagent/data/voice_lexicon.json`

2. `crates/goose-server/src/voice/pronunciation_coaching.rs`
   - Teach flow: user says "that word, it's pronounced X not Y"
   - Store both pronunciations
   - On next STT: if ambiguous word, prompt user to clarify which one they meant

3. Tests: homograph stored & retrieved on next turn

**Verification:**
- [ ] Teach homograph → stored
- [ ] Next turn: ambiguous word → he asks which pronunciation

**Done when:**
- [ ] Tests pass
- [ ] Settings show enrolled homographs

---

### TASK-N9: Empty STT Handling
**Worker:** Cheap subagent  
**Branch:** `goal/voice-n9-empty-stt`  
**Scope:** Distinguish VAD abort from empty transcript  
**Estimated effort:** 3–4 hours  
**Depends on:** TASK-N1 (VAD abort signal)

**Deliverables:**

1. `ios/PermagentMobile/PermagentMobile/VoiceVAD.swift`
   - Distinguish: VAD abort vs. real-audio-but-empty-transcript
   - Signal difference via frame type or flag

2. `crates/goose-server/src/routes/voice.rs`
   - On VAD abort: silent reconnect (back to Listening, no error)
   - On real-audio-empty: only then toast error

3. Tests: VAD abort → no error toast; real-audio-empty → error toast

**Verification:**
- [ ] Short noise or reconnect: silent (no error)
- [ ] Real speech silence: error toast only

**Done when:**
- [ ] Tests pass
- [ ] On device: quick noise → no toast, back to Listening

---

## Delegation Strategy

### For Each Worker
1. **Primary worker:** Gets the task spec (from above) in a goal branch
2. **Tests first:** Write failing tests; verify they fail on main
3. **Production fix:** Green the tests
4. **Review gate:** Decision Inbox approval before landing to main
5. **Integration:** Once all phase tasks land, do integration testing

### Parallel Work
- **Phase 1:** N0 (tests) + Origin-N1 (routing) can run in parallel
- **Phase 2:** N1 (VAD), Origin-N2+N3 (prompts), N2 (isolation) can run in parallel
- **Phase 3:** N3 (enrollment), Origin-N4+N5 (tools) can run in parallel
- **Phase 4:** N4, N6, N8, N9 can run in parallel (independent)

### Cost Optimization
- **Main loop (Claude Code/Codex):** Hard reasoning (DAG design, DSP tuning, enrollment model, tool gating)
- **Cheap subagent:** Mechanical tasks (N6 markdown strip, N8 homographs, N9 empty-STT handling, N0 latency-report parsing)

---

## Handoff Checklist

- [ ] Master DAG reviewed with user (this file)
- [ ] Phase 1 tasks assigned to Codex/Claude Code
- [ ] Worker knows: branch naming, review gate, tests-first discipline
- [ ] Integration test plan drafted (cross-device testing on real iOS device)
- [ ] Success metrics aligned (Henry not stuck on Listening, enrollment works, knows where he is)
