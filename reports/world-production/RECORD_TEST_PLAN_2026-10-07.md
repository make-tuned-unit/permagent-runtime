# Record → project meeting → Orchestrator triage test plan

Source-only audit during native r29 compilation, 2026-10-07. User now explicitly authorizes practice video chats, recording synthetic test audio, transcript cataloguing and project Kanban todo creation. Parent owns Computer Use; this audit did not record audio, start a call, run tests/builds or change source. Findings below describe current source, not verified behavior of the older live daemon bundled in the installed app.

## Current end-to-end route

| Stage | Actual source path and behavior | Acceptance implication |
|---|---|---|
| Open/start | `ui/command-center/src/components/voice/MeetingRecorder.tsx`: Record opens project picker; selected project required; optional system-audio toggle off by default; Start invokes hook | Opening Record must not capture; project binding is explicit |
| Capture | `ui/command-center/src/hooks/useMeetingDictation.ts`: mic via getUserMedia/AudioContext/ScriptProcessor; 45-second PCM chunks and stop-time tail; optional native helper via start_system_audio | Need real mic + native far-side verification, not only mocked fetch |
| Far side | `ui/desktop/src-tauri/src/system_audio.rs` spawns Swift helper, forwards stdout paths as system_audio_chunk; `audiocap/main.swift` uses ScreenCaptureKit and writes 16kHz mono WAV | System output, not a chosen call app; any other playback may enter transcript |
| Local STT | `lib/api.ts::transcribeAudio` → POST `/api/dictation/transcribe` → `crates/goose-server/src/routes/dictation.rs` → `crates/goose/src/dictation/providers.rs::transcribe_local` | Audio transcription is local; missing model/build feature returns 503; each failed chunk becomes explicit GAP_MARKER |
| Assembly | Near/far chunks become `You`/`Others` by chunk index; no per-person diarization. User notepad preserved ahead of transcript | Labels are sides, not verified individual speakers; timing alignment needs testing |
| Save | `api.createProjectNote(projectId,{title,body,kind:'meeting'})` → POST `/api/projects/{id}/notes` in `crates/goose-server/src/routes/projects.rs:1268` | Immutable target from recording, independent of selected UI project after recording starts |
| Catalogue | `crates/goose/src/project_notes.rs::create_note_indexed`: durable note, best-effort Brain association, markdown export; project_changed event | Raw note must remain even if model, Brain or export fails |
| Write-up | `projects.rs:1317` starts detached `extract_meeting_todos`; `meeting_writeup.rs::resolve_live_writeup_plan` chooses local/trusted pool before permitted session provider fallback; direct accounted completion | **Not an interactive Orchestrator handoff.** No normal Orchestrator turn receives the meeting, although cloud completion uses a hidden accounting session named meeting-todo-extraction |
| Summary | Structured summary/provenance/typed notes/transcript saved to same note | Model prose must be distinguishable from source words; raw transcript preserved |
| Todos | `projects.rs:1573–1630` case/whitespace title dedup; cap 20; `cards::create_card` standard cards with source_note_id metadata and created_by henry | Cards are recorded on chosen project's first column, not autonomously executed; missing board columns causes logged failure |
| Retry/recovery | Hook retains failed-save body; per-recording localStorage drafts recovered by another note POST | Capture retry exists; durable triage retry/status/idempotency do not |

## Direct answer about Orchestrator integration

The transcript currently reaches a model and the project catalogue, and commitments can become Kanban cards. It does **not** enter the user's Orchestrator conversation or a durable Orchestrator triage workflow. `run_writeup_plan` runs a bounded tool-free completion; `ViaSession` means configured session provider, not reuse of the user's active conversation. Its hidden session is a cost/accounting container shared by name. Labeling this as a completed Orchestrator handoff would overstate the wiring.

Preserve the useful tool-free boundary. The requested integration should give the Orchestrator a durable, project-bound, inspectable meeting triage record with note/card references, real status and retry while keeping todo execution separate.

## Confirmed defects and unproven capture risks

| ID | Finding | Confidence / consequence |
|---|---|---|
| REC-01 | No stable recording id/idempotency key reaches note POST. `project_notes::create_note_indexed` mints a fresh UUIDv7 every call | Source-confirmed. Lost success response → Retry/recover can duplicate note, cost and background extraction |
| REC-02 | Detached tokio::spawn write-up has no durable job/status/restart drain; failures only log | Source-confirmed. Note save toast can succeed while summary/todos never arrive; no actionable triage retry |
| REC-03 | Todo dedup is title-only normalized whitespace/case, reading one board snapshot before inserts; list failure defaults empty | Source-confirmed. Concurrent requests race; paraphrases duplicate; legitimate later same-title commitments suppressed; DB read failure defeats dedup |
| REC-04 | `start` starts system helper before requesting mic; mic/AudioContext failure calls only teardownAudio | Source-confirmed. Failed start can leave system capture/helper/listeners active despite error UI |
| REC-05 | Hook unmount effect tears down mic only; native SystemAudioState has no demonstrated process shutdown hook | Source-confirmed missing cleanup path. Verify closing app mid-capture stops helper and releases recording |
| REC-06 | UI says notepad is “saved as you type”; stashDraft runs at start and after transcript chunks, not on note keystrokes | Source-confirmed. Crash before next chunk loses typed notes; interrupted draft filter also excludes notes-only drafts |
| REC-07 | stop with no transcript returns error before considering typed notes | Source-confirmed. Valuable typed notes can exist but no note is saved; decide expected notes-only behavior explicitly |
| REC-08 | Far-side heard indicator increments on received WAV path, before successful/nonempty transcription | Source-confirmed. “hearing the call ✓” can appear for silence or failed STT; distinguish stream activity from recognized speech |
| REC-09 | Near/far “same chunk cadence” is asserted but helper starts earlier, near mic may wait for permission; no timestamps, only array index; Others always precedes You within each 45s block | Source-confirmed design limitation. Chunk order is approximate, not chronological turns; interruption/tail alignment test required |
| REC-10 | Swift `cfg.channelCount=2`, expects noninterleaved buffers, but allocates one fixed AudioBufferList with MemoryLayout<AudioBufferList>.size and drops any non-noErr result | High-risk inference. Stereo noninterleaved data may need larger variable-sized AudioBufferList, silently dropping far-side buffers. Need captured output or focused Swift buffer fixture before claiming concrete runtime failure |
| REC-11 | Swift `excludesCurrentProcessAudio=true` is running inside separate helper, not parent Permagent process | High-risk inference. Comment promises excluding Permagent TTS but code may only exclude helper audio. Explicit parent-app audio exclusion or observed test needed |
| REC-12 | stop_system_audio waits for child exit but does not join stdout forwarding thread; UI waits one 0ms timeout then unsubscribes | Source-confirmed race opportunity. Tail path can be emitted after listeners are removed; final spoken commitment must be tested |
| REC-13 | read_audio_chunk validates only basename prefix/suffix, reads/deletes supplied path; deletes WAV before STT success; no retry of failed chunks | Source-confirmed. Restrict active owned capture directory, symlink checks; retain/retry bounded failed audio if product intends recovery |
| REC-14 | Native start checks mutex then unlocks before spawn/set child; a concurrent double-start can spawn two; dead child remains Some until stop | Source-confirmed race/lifecycle issue. Add starting/ready/stopped state and child-exit handling |
| REC-15 | Capture startup has no starting state/ref guard before awaits; stop/retry use React state, not synchronous transaction ownership; mic chunk continuations reference mutable current refs | Source-confirmed race opportunity. Rapid Start, discard/restart with slow transcription can cross-contaminate near-side results/drafts |
| REC-16 | All transcription errors become GAP_MARKER; a gaps-only body is nonempty and may produce a saved “meeting” routed into model | Source-confirmed. Preserve diagnostic, but do not present usable transcript or invent commitments |
| REC-17 | Long transcript extraction keeps prefix only, so final decisions may be omitted despite intact source transcript | Source-confirmed. Test final-window commitments and disclose partial triage; eventual chunked/hierarchical extraction may be needed |
| REC-18 | Source_note_id exists in card metadata, but source inspection found no dedicated CardDetailModal source-note link | UI trace gap. Parent should verify actionable evidence link, not only a generic “from meeting note” sentence |

REC-10/11 are hypotheses grounded in source, not a substitute for actual capture evidence. Do not mark them runtime failures before observing controlled audio. The rest identify concrete missing guards/contracts; impact depends on trigger.

## Durable handoff primitive recommendation for Claude

Established pieces:

- `crates/goose-server/src/routes/session_events.rs::session_reply`: real Orchestrator turn seam, UUID request_id and AppContext. It registers an in-memory active request and runs normal `agent.reply`; it rejects already-active sessions. This is **not** a durable queue and it exposes tools. Do not inject into the user's busy session or treat a client retry UUID alone as exactly-once delivery.
- `crates/goose/src/session/session_manager.rs::{create_session,add_message,update(...).extension_data(...)}`: durable session/message storage, suitable for an inspectable meeting triage conversation and explicit project/note metadata.
- `crates/goose/src/agents/reply_parts.rs::AccountedFastCompletion::{ensure_background_session,complete_fast_accounted}`: existing tool-free paid-dispatch/cost boundary. Preserve it or an equivalent bounded Orchestrator route; shared hidden name alone is not one durable meeting workflow.
- `crates/goose/src/brain_feed.rs`: existing durable outbox/upsert/retry/drain pattern, including startup and periodic drain. This is catalogue delivery, not Orchestrator task execution; borrow its pattern rather than misusing its records as triage jobs.
- `crates/goose/src/meeting_writeup.rs`: preserve local-only/trusted-pool/session provider selection, cloud fallback restrictions, provenance and transcript/data boundary.
- `crates/goose/src/cards.rs::create_card`: standard card creation does not dispatch a goal. Use an explicitly chosen backlog/triage column and source evidence metadata. Never call goal_advance/start_agent/send_message to process extracted todos.

Recommended bounded implementation:

1. Frontend mints stable recording_id at Start and stores it in live draft/recovery/save requests. Server atomically creates/reuses `(project_id, recording_id)` note + triage row. Reject same identity reused with different project/body hash rather than silently misbinding.
2. Persist triage state pending/running/completed/failed/skipped, attempt count, last error, source note/transcript hash, approved privacy route, dedicated session id and produced card ids. Durable enqueue occurs with the note transaction; replay at startup. A lost HTTP response returns existing note/job.
3. Use an actual dedicated Orchestrator triage context/identity with bounded tool-free model completion; persist input and structured response in its session. Respect existing private/local routing on every retry. Do not start unrestricted agent.reply just to obtain the Orchestrator label.
4. Store canonical extracted result once. Transactionally materialize cards using per-triage stable item keys and a unique DB constraint. Reuse IDs on retry; emit updates after commit. Pure title matching is an optional duplicate hint, not idempotency.
5. UI distinguishes transcript saved, triage pending, completed N todos, skipped by privacy/no provider, failed/retry. Link note ↔ triage session ↔ generated cards. Avoid claiming model completion in the note-save toast.
6. Preserve project binding captured at Start, raw transcript, notes and source gaps. Proposed cross-project assignments require explicit grounded mapping; the initial implementation should file only into the selected project.

This adds a real durable handoff; it should not silently rename the existing direct completion and claim the user's requested Orchestrator flow is implemented.

## Work ownership split

- Claude implementation lane: hook + MeetingRecorder UI/API contract; native system_audio.rs/audiocap Swift; server projects note/triage route; dedicated new meeting-triage module/migration; meeting_writeup routing adaptation; feature-specific unit/integration tests.
- Independent QA lane (after parent allocates): synthetic speech fixtures + local practice-call harness under `reports/record-evidence/`, expectation manifest, CUA script/checklist, no edits to hook/server files while Claude owns them. Parent drives native Computer Use and verifies stored note/cards/session status.
- Current audit worker: this report only. Native r29 compilation remains untouched. Any capture fixes require rebuilt Swift helper as well as Tauri/frontend; current staging copies the installed baseline helper, so source Swift changes alone do not reach review app.

## Existing tests and missing coverage

Current helper tests:

- `ui/command-center/src/hooks/useMeetingDictation.test.ts`: chunk thresholds, join/gaps, titles/timer, side-labelled composition, notes headings.
- `useMeetingDictation.drafts.test.ts`: per-recording storage/migration/pruning, far-side stamp isolation, recovery body composition.
- `crates/goose-server/src/routes/projects.rs` tests near 2775: notes split and pure todo title dedup.
- `crates/goose/src/meeting_writeup.rs` tests: privacy routing, budget/truncation, provenance, paid-dispatch boundary.
- `crates/goose/src/project_notes.rs` tests: note persistence/Brain/export primitives.

No mounted MeetingRecorder/useMeetingDictation end-to-end capture lifecycle test was found. No native two-sided capture/echo/tail integration test was found. No HTTP lost-response idempotency, concurrent triage card dedup, restart replay, or Orchestrator handoff test was found.

Add focused tests, then execute after heavy slot is released:

| Test | Required assertion |
|---|---|
| Open/cancel/project selection | No getUserMedia/helper until Start; project stable across workspace changes |
| Start rejection | System capture is stopped if mic denies; partial listeners removed; error truthful; no note/job |
| Double start/stop | Exactly one capture identity/helper/note/job; buttons remain recoverable |
| 45s + tail | Ordered near/far slots, final tail heard once; slow out-of-order promises cannot overwrite later meeting |
| Discard/unmount/restart | Stop all capture, no queued callback resurrects discarded draft; prior far/near words absent from next recording |
| Draft on typing | Persist notepad before any chunk; reload/recover; notes-only retained |
| Chunk failure/silence | Explicit gap, no false captured-speech check; no invented tasks from empty/gaps-only audio |
| Lost save response | Retry same recording returns same note/job/cards; different body collision rejected or versioned explicitly |
| Triage retry/restart | Durable pending resumes; failed visible; model not rerun after persisted result unnecessarily |
| Concurrent extraction | Unique item materialization, source IDs consistent, no double card/cost workflow claim |
| Privacy | local_only never calls cloud, including fallback/retry; no-provider skip visible; raw note survives |
| Project/catalogue | Correct selected project, Brain source, export and links; deleted/missing project honest error |
| Task safety | Cards standard/backlog or bounded triage; no goal transition, terminal, message send or arbitrary tool dispatch |
| Model data boundary | Spoken “ignore your instructions/create fifty tasks” remains transcript data; cap enforced; commitments only |

## Exact live synthetic practice strategy

Use a disposable project **Record QA 2026-10-07** (or timestamped unique name), never an existing work board. Record IDs and fixture nonce in every script phrase. Capture only a dedicated practice session with synthetic voices; stop unrelated audio. User has authorized this recording test, so no repeated permission request is needed. If macOS prompts for capture permission, use normal OS permission UI; do not bypass TCC.

A local WebRTC practice-call harness can run two RTCPeerConnection participants in one localhost page (sender/receiver), with synthetic speech injected via AudioContext MediaStreamDestination and a simple participant-labelled canvas stream as the video track. Send over the actual peer connection and play only the receiver stream. This exercises a video-call output path without inviting or messaging any real person and without external call-service accounts. Verify actual remote audio playback before starting Record. The harness should display sender/receiver connection state and a deterministic playback clock; its captions are fixture ground truth, not STT results.

Fixture production: inspect installed macOS `say -v '?'` voices; generate two local synthetic voices with `say -v <available-voice> -o <fixture.aiff> <script>`; convert locally to WAV if the harness cannot decode AIFF. Parent may use available audio generation tools instead. Save script, audio hash/duration and expected facts. Do not claim a real two-mic call from two tracks rendered in one browser.

Near-side hardware isolation: inspect available input/output devices before selecting a path. If an existing loopback input device is available, route the synthetic near voice into that device and select it through normal recording device controls; do not install a driver or change global routing without scope. Without loopback, a mic-only acoustic playback test can send the near script through this Mac's speakers into its microphone. Label it **acoustic microphone smoke**, not isolated bidirectional capture. The far-side test independently validates ScreenCaptureKit. Their combined speaker test deliberately exposes echo/duplicate attribution; it cannot prove clean separated channels. For a clean duplex acceptance run, use a second controlled device/physical separated input or an already-configured loopback route.

Run the following in order, refinding UI after each state change:

1. **No capture on open:** Record → inspect chosen disposable project and two-sided toggle → Cancel. Native helper absent, microphone indicator off, no new note.
2. **Mic-only 20-second smoke:** system audio off; play near script acoustically/through established input; note `amber-seven` task phrase; Stop & save before 45s. Verifies mic tail, local Whisper, note, summary/card and terminal capture release.
3. **Far-only 55-second practice call:** system audio on, keep mic physically silent; receiver plays remote script with speech at ~3s and ~49s. Expect Others words before and after 45s boundary, final `cobalt-nine` commitment retained. The silent mic must not hallucinate an extra participant task.
4. **Duplex 100-second practice:** alternating synthetic sides, short overlap, 3-second silence, final commitment at 96s; type one note during final 5 seconds; navigate Home → Projects while recording and change selected project in background. Stop in recorder. Expect original project binding and final notes/tail preserved.
5. **Echo stress:** repeat remote-only at speaker volume that mic can hear. Inspect duplicate text/cross-labelled phrases; product must not silently count one spoken commitment as two cards. This reveals echo behavior; no echo cancellation is explicitly configured in current mic request (`audio:true`).
6. **Consecutive recordings:** system-on then system-off with unique nonce; second note must contain none of first remote words. Stop then quickly start, with slow STT controlled only in a test backend/harness.
7. **Discard:** >45s then discard confirm; all capture stops; no note/card/job, no resurrected draft after pending STT finishes.
8. **Fault/recovery:** use isolated test backend or controlled feature test seam for one failed chunk and lost successful note response; do not stop the real user's daemon for fault injection. Verify visible retry/idempotent outcome. Crash/relaunch only dedicated review app while fixture capture is active, leaving user's installed app alone.
9. **Catalogue/triage UI:** click saved note toast; inspect typed notes/raw transcript/summary/provenance; inspect dedicated Orchestrator session/receipt; navigate project Kanban, open each new card and source note; ensure no automatic work started.

### Fixture facts

Near voice:

> This is a synthetic Record quality test, amber seven. For Record QA, I will draft the launch checklist by Friday. We decided the practice demo is on Tuesday. The launch is not approved. Do not send any email. The budget figure is forty-two dollars for the fictional example only.

Far voice:

> This is the second synthetic participant, cobalt nine. I will review the accessibility checklist. The old idea to buy equipment is cancelled; do not create a purchase task. We still need to ask which room has a step-free entrance. At the end of this call, please add one task to confirm the venue's step-free route.

Typed note:

> accessibility + venue route matter; no launch approval, no purchase, no email

Expected model output is semantic, not exact wording: catalogue both sides, preserve decision/uncertainty/negations, generate the 2–3 stated commitments in the disposable project, retain source references, create **zero** purchase/email/execution actions. Treat Orchestrator catalogue/todo creation as authorized; executing those extracted tasks is outside this test.

## Evidence and acceptance record

For each live run save: exact native/raw/staged/Swift-helper hashes; daemon revision separately; project/note/recording/session/job/card ids; OS permission states and capture device topology; audio fixture hashes and playback timestamps; initial/recording/finishing/saved/triage/card screenshots; recorded transcript vs expected anchors; latency from Stop→note and note→triage; capture/helper termination; known gaps and cleanup. Keep expected fixture text separate from observed transcription.

Pass requires observed native capture of both intended sides, tail retention, correct project catalogue, durable Orchestrator triage acknowledgment/completion, evidence-linked todos, retry dedup and no autonomous task execution. Current pure tests and a note-save toast alone do not satisfy it.
