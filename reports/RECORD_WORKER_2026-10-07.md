# Record worker report — 2026-10-07

Worker: claude-opus-5-5, single worker. Worktree `wq-record-20261007`, branch `wq/record-20261007` from b55bf214.
Scope: recorder frontend hook/component and focused tests only. No commits, push, native/cargo builds, daemon access, recording or UI control. Orchestrator and backend are unchanged.

## Files

| File | Change |
|---|---|
| `ui/command-center/src/hooks/useMeetingDictation.ts` | lifecycle fixes (below) |
| `ui/command-center/src/components/voice/MeetingRecorder.tsx` | tooltip fix; two-step Dismiss when an unsaved transcript would be deleted |
| `ui/command-center/src/hooks/useMeetingDictation.lifecycle.test.tsx` | **new**: 20 behavioural lifecycle tests |
| `ui/command-center/src/hooks/useMeetingDictation.test.ts` | +4 pure tests (`meetingNoteBody`, `micStartError`) |
| `ui/command-center/src/components/voice/MeetingRecorder.test.tsx` | **new**: 2 tooltip tests |

Public hook API: no breaking change. The return shape is the same. New exports: `NO_SPEECH_MARKER`, `NOTES_STASH_DEBOUNCE_MS`, `meetingNoteBody`, `micStartError` and `recordButtonTooltip` (component file).

## Checks (run from `ui/command-center`)

| Command | Exit | Result |
|---|---|---|
| `npx vitest run src/hooks/useMeetingDictation src/components/voice/MeetingRecorder` | 0 | 4 files, 58/58 passed |
| `npx tsc --noEmit -p .` | 0 | no output |
| diff grep for developer name / legacy agent names | — | no hits |

I did not run the full vitest suite, as instructed.

## Defects fixed (each is pinned by a lifecycle test)

1. **System audio started before the mic (confirmed concern).** If the mic was refused, the ScreenCaptureKit helper kept recording with no recording behind it. Each chunk it sent then stashed a draft under the *previous* recording's `startedAt`, which overwrote a stranded transcript. Now the mic and audio graph are acquired first. `targetRef` and `startedAt` are set only once the mic is live, and the helper starts last.
   - On mic or AudioContext failure, the mic is torn down, the recording id is retired and the target is cleared.
   - Tests: `never starts the helper when the mic is refused`, `releases the mic when the audio graph fails…`.
2. **Mic error text.** Every failure, including a missing device or an AudioContext error, was reported as "Microphone access was blocked". `micStartError` now gives separate messages for a refusal, a missing device and other failures.
3. **Listeners leaked when `start_system_audio` failed.** They were registered first and never removed. The failure path now unlistens them; the mic keeps recording and the error appears on the panel.
4. **A helper left running by a reloaded webview.** The native `start_system_audio` returns OK when a child already exists, so the new recording inherited a stale capture. The renderer now calls `stop_system_audio` before starting.
   - This is a defensive fix: I reasoned it from the native code (`system_audio.rs:59-67`). No test reproduces it end to end.
5. **Unmount mid-recording left the helper running.** Only the mic was torn down. Unmount now retires the recording id and stops the helper. A start still waiting on the helper sees the retired id and stops it too.
6. **Double Stop saved two notes, and double Retry did the same.** The guards read `state`, which is stale within a frame, so both calls passed. Added `stoppingRef`, `savingRef` and `startingRef`, and both save paths now go through `attemptSave`. Double Start likewise opened two microphones.
7. **A stale far-side listener filed audio into the next, mic-only recording.** A mic-only recording never re-stamps the far stash, so the old stamp still matched. Both sides were affected:
   - **Hook:** the stale listener pushed into the old stash, bumped `farChunksHeard` (so the indicator could claim "hearing the call") and transcribed the old call's audio.
   - **Note:** the body itself stayed correct.
   - **Fix:** the listener now checks the live `recordingIdRef` as well as the stamp.
   - Test: `a mic-only recording never inherits the previous call's far side or project`.
8. **User notes were lost in three places:**
   - **Silent or dead-mic stop:** "No speech was transcribed — no note was saved" threw away what the user had typed. The notes are now saved under `## Your notes`, with `## Transcript` containing `NO_SPEECH_MARKER`. When there is neither speech nor notes, nothing is saved and the empty draft is cleared.
   - **Typing between chunks:** the panel said "saved as you type", but the draft was only written every 45 s. A crash lost up to 45 s of typing. The draft is now written 400 ms after typing stops, and once more just before the save attempt.
   - **Notes-only crash drafts:** these were hidden by the recovery filter. The filter now counts notes.
9. **Starting a new recording after a failed save hid the old transcript.** Its draft was still in storage but stayed invisible until the next launch. It is now moved onto the recovery card immediately.
10. **Dismiss after a failed save deleted the whole meeting in one click.** It now needs a second click when an unsaved transcript exists ("Delete transcript — sure?").
11. **The tooltip said "Stop recording and save the note" while recording,** but the button only shows the panel. It now reads "Show the recording panel — Stop & save is there".

Unchanged on purpose:
- Speaker labels stay per side (`**Others:**` / `**You:**`), and mic-only stays single-voice. A test asserts that no other label appears; nothing claims named speakers from mixed system audio.
- The body headings (`## Your notes` / `## Transcript`) match the daemon's `split_meeting_body` and are unchanged.
- Notes are still saved with `kind: 'meeting'` to the chosen project. Tests assert the project id and kind for live, retry and recovery saves.

## Not verified

- **Anything native or real:** no native build, no real getUserMedia or ScreenCaptureKit, no actual call. The parent's Computer Use and practice-call run is the real test.
- **Reverse check against the original hook:** shell writes into the worktree need approval, so I did not run the new tests against b55bf214's hook. Fixes 1, 3, 5, 6, 7, 8 and 9 each have a test whose assertion fails under the old code paths I read, but I did not observe those failures.
- **Component rendering:** the panel itself (two-step Dismiss, docking) is not render-tested. Only `recordButtonTooltip` is tested.
- **Chunk alignment:** the mic now starts before the helper, so the far-side chunk windows begin slightly later than the mic's (spawn latency; up to about 3 s if a stale helper must be stopped first). Interleaving by chunk index was already approximate; this shifts it a little.
- **Leftover temp files:** a chunk the helper flushes after a discard or unmount is ignored. Its WAV stays in `/tmp/permagent-audiocap-*` because only `read_audio_chunk` deletes files. That is a native cleanup gap I left alone.

## Gaps for the next assignment (Orchestrator handoff), from a read-only look at `projects.rs` / `meeting_writeup.rs`

1. **No durable handoff.** `create_project_note_handler` (`projects.rs:1317`) runs `extract_meeting_todos` as a detached `tokio::spawn`. A daemon restart or crash after the save loses the triage, and nothing retries. The note row has no pending/done/failed state.
2. **The Orchestrator is not involved.** One fast-model pass inside the route extracts todos and files them only on the project picked before the call. There is no cataloguing, and no routing to *other* relevant kanbans. That is the gap between this and the requested "Granola plus project/AI triage".
3. **No outcome reaches the user.** The client toast says "Writing up the notes … now". Skip (`meeting_writeup_local_only` with no local route), model failure and "no action items" are only logged. Only the note rewrite emits `project_changed(notes)`.
4. **No idempotency key.** A live save plus a recovery of the same meeting creates two notes. Cards are deduped by title, notes are not. The client has a per-recording identity (`startedAt` ISO) but does not send it.
5. **The extraction prompt doesn't know about the labels.** It is never told that `**Others:**` is one mixed stream of everyone else, so a model may invent names for attendees. It also doesn't know `[… a segment could not be transcribed …]` or the new `[no speech was transcribed]`.
6. **Truncation keeps the start.** It keeps the *first* N characters, so decisions at the end of a long meeting are the part dropped.
7. **Misleading comment.** `meeting_writeup.rs:64` says "Intentionally broken: always prefer session…", but the code prefers mesh/local. The comment should be removed (not touched; read-only).

### Expected contract (a proposal for the parent's audit worker to confirm or replace)

**Client → daemon:** keep `POST /api/projects/{id}/notes` and its body and headings unchanged, and add an optional `meeting` object:

```
meeting: {
  recording_id,   // client stable id: startedAt ISO + random suffix
  started_at,
  ended_at,
  sides: ["mic"] | ["mic", "system"],
  failed_chunks,
  source: "live" | "recovered"
}
```

**Daemon behaviour:**
- Persist the note together with a triage job in one transaction, keyed by `recording_id` (an idempotent replay returns the existing note and job).
- Respond `{ note, triage: { job_id, state: "queued" } }`.

**Orchestrator:**
- Consumes queued jobs durably, surviving restarts with bounded retries.
- Catalogues the note.
- Proposes or files cards on the origin project and on other relevant projects, each card citing `note_id` and `recording_id`.
- Is told explicitly that "Others" is a mixed, unattributed stream.

**Events:** emit `meeting_triage_changed { job_id, note_id, state: queued|running|done|skipped|failed, cards: [{project_id, card_id, title}], reason? }`.

**UI (next assignment):** the recorder toast or panel follows that event: "3 to-dos filed on X, 1 on Y", "triage skipped: local-only mode, no local model" or "triage failed — retry". Existing tasks and projects are only appended to, never edited, unless the Orchestrator returns a decision for review.

## Practice-call checklist for the parent (native)

1. **Mic denied:** with system audio ON, deny the mic. The panel shows "Microphone access was blocked", and `ps` shows no `permagent-audiocap` process.
2. **Both sides:** the synthetic call yields a note with both `**Others:**` and `**You:**`, and the last stretch of the call (the chunk flushed at stop) is present.
3. **Save failure:** stop the daemon before Stop & save. The panel shows Retry, the draft survives a quit and relaunch, and Retry produces exactly one note.
4. **Recording after a failed save:** after a failed save, start a new recording. When it ends, the old transcript is on the recovery card.
5. **Silent recording with notes:** the notes are saved with `[no speech was transcribed]`.
6. **Tooltip:** while recording, hovering the rail Record button reads "Show the recording panel — Stop & save is there".

---

# Round 2 — recordingId, triage status, stop-return tail (2026-10-07)

Same worktree, branch and model (claude-opus-5-5). Scope: Record types and functions in `lib/api.ts`, the new `lib/systemAudio.ts`, `hooks/useMeetingDictation*` and `components/voice/MeetingRecorder*`. No backend, native, integration-worktree, store or navigation edits. No commits.

Contract source: `world-production-20261006/reports/world-production/RECORD_BACKEND_DESIGN_2026-10-07.md`. Receipt shape read from `crates/goose/src/meeting_triage.rs` (`MeetingTriageReceipt`, camelCase; snake_case status).

## Files (round 2)

| File | Change |
|---|---|
| `ui/command-center/src/lib/api.ts` | `MeetingTriageStatus` and `MeetingTriageReceipt` types. `createProjectNote` now takes an optional `recordingId` and returns `ProjectNote & { triage? }`. New `getMeetingTriage` and `retryMeetingTriage`. |
| `ui/command-center/src/lib/systemAudio.ts` | **new** wrapper around the helper commands and events (`systemAudioAvailable`, `startSystemAudioCapture`, `stopSystemAudioCapture`, `readSystemAudioChunk`, `listenSystemAudio`, `normalizeStopResult`) |
| `ui/command-center/src/hooks/useMeetingDictation.ts` | stable recordingId, frozen draft body, one dedup intake for far-side chunks, triage following |
| `ui/command-center/src/components/voice/MeetingRecorder.tsx` | triage card and `triageStatusLine`; the save toast no longer implies the write-up is done |
| `ui/command-center/src/hooks/useMeetingDictation.lifecycle.test.tsx` | +16 tests (36 total) |
| `ui/command-center/src/hooks/useMeetingDictation.test.ts` | +4 pure tests; now `@vitest-environment jsdom` |
| `ui/command-center/src/components/voice/MeetingRecorder.test.tsx` | +5 `triageStatusLine` tests |
| `ui/command-center/src/lib/systemAudio.test.ts` | **new**, 4 tests |

## Checks (round 2, from `ui/command-center`)

| Command | Exit | Result |
|---|---|---|
| `npx vitest run src/hooks/useMeetingDictation src/components/voice/MeetingRecorder src/lib/systemAudio` | 0 | 5 files, 87/87 passed |
| `npx tsc --noEmit -p .` | 0 | no output |
| name-guard grep over the owned files | — | only "t**hank**s" inside test strings (substring false positive) |

## What changed

**Stable recordingId**
- Minted once, after the mic is live (`crypto.randomUUID`, with a fallback), and stashed in every draft write.
- The live save, Retry and recovery all send that same id. A retry after a lost response reuses `unsavedRef.recordingId`; nothing regenerates it.
- At stop, the exact title and body are frozen into the draft (`MeetingDraft.title/body`) before the save can fail. A recovery after a quit therefore re-sends the same bytes under the same id, so the daemon's body-hash check sees the same note instead of a 409.
- Legacy drafts with no id recover under `legacy-<startedAt>`, which is the same on every attempt.

**Note saved vs. triage**
- A save no longer implies the write-up is done. The toast says only "Saved to "X" — click to open the note."
- A separate card (`role="status"`, idle only, yields to the recovery card) says "Meeting note saved → Project" and gives one status line from the receipt:
  - queued, or running;
  - "Triage done — N to-do card(s) on the board" (N = `cardIds.length`), or "no to-dos found";
  - "Triage failed: <lastError>" with **Retry triage**;
  - skipped (no permitted model) with **Retry triage**;
  - "didn't report triage status" (older daemon, or 404);
  - "still … — check again" with **Check again** once polling has given up.
- Links use existing navigation only: **Open note** (`focusProjectNote`), **Open board** (`openCardOnBoard` with the first card id), **Open triage session** (`switchToSession(sessionId)`, the same call Goals, Build and Automate use).

**Polling**
- Backoff of 2, 3, 5, 8, 13, 20 then 30 s, repeating the last delay.
- Stops when the status settles, on a 404, after 40 polls, or after 3 consecutive errors (the last two show **Check again**).
- A generation counter cancels the loop on a new save, on dismiss and on unmount.
- An older daemon that returns no receipt is never polled.

**Last-saved recovery**
- `permagent-meeting-last-saved` in localStorage (one slot, 24 h TTL) is written only when a receipt came back.
- On mount it is fetched immediately and polled if not yet settled. Dismiss clears it.

**Stop-return tail**
- `stopSystemAudio` takes the intake before calling `stop_system_audio`. It waits one tick so any flushed events dispatch, feeds the returned paths through the **same** intake, and only then unsubscribes; `stop` then awaits the far queue.
- The intake dedupes by path (`FarSideStash.seen`) and is closed over its own stash and recording generation, so a stale callback cannot reach a later recording.
- An older native that returns `null` falls back to the event path alone.
- The orphan reset at Start, discard and unmount call `stop_system_audio` without taking the returned paths.

## Tests added (all in the focused run above)

**recordingId** (lifecycle file)
- minted, stashed and sent;
- a new id per recording;
- lost response → retry with the same id and bytes;
- failed save → quit → remount → recover with the same id, body and title;
- legacy draft → `legacy-<startedAt>` on both attempts.

**Tail** (lifecycle file)
- event 0000, plus event 0001, plus return `[0000, 0001, 0002]` → read exactly `[0000, 0001, 0002]` once each, interleaved in order;
- older native returning `undefined` → the tail arrives via the event;
- paths returned by the orphan reset at Start are not taken;
- discard takes no returned paths;
- a stale intake from recording A cannot add to recording B.

**Triage** (fake timers; the save goes through recovery so no audio timers interfere)
- pending → running → completed with cardIds, then no further polls;
- failed → Retry → POST retry, then polling resumes to completed;
- a missing receipt is reported unavailable and never polled;
- 3 errors → stalled with the error shown → Check again recovers;
- unmount stops polling, the next mount resumes from last-saved, and Dismiss clears it;
- a 404 marks it unavailable and polling stops.

**Pure**
- `normalizeStopResult`, `triageStatusLine`, `draftRecordingId`, `triagePollDelay`, `readLastSaved` TTL.

## Found while testing

- Two concurrent dynamic `import('@tauri-apps/api/core')` calls in vitest 1.6 handed the second caller the **real** module, bypassing the mock: a chunk read concurrent with the stop then failed and became a gap marker. The wrapper now loads the module once and shares that promise. In the app this only saves repeated imports; in tests it makes the boundary mock deterministic.

## Contract assumptions to confirm (integration)

- **Retry route:** I used `POST …/recordings/{id}/triage/retry`, as the brief states. The integration `projects.rs` I read (line 2231) still routes the retry as `POST …/triage` with no `/retry`. If the backend fix doesn't add `/retry`, Retry triage will 404/405.
- **Missing recordingId:** the brief says a meeting save without one stays accepted, while the integration handler still returns 400. The frontend always sends one, so this only affects other or older clients.
- **Skipped wording:** text for `skipped` assumes the daemon's only reason is "no permitted model" (local-only with no local route, or no session model), as in `resolve_meeting_writeup_plan`.
- **Card count:** N is `cardIds.length`, the cards this triage materialized. If a retry re-materialization reports cards that already existed, N counts them too.

## Not verified (round 2)

- No native run: the return value of the new `stop_system_audio` is modelled from the brief, not observed.
- No render test of the triage card; only its status text is tested. Its layout at real size, and opening a hidden triage session through `switchToSession`, need the parent's native/Computer Use pass.
- Full vitest suite not run, per the brief.
- `useMeetingDictation.test.ts` and `.drafts.test.ts` still import the real `lib/api` (pre-existing), which logs a harmless daemon-token warning in tests.

## Native checks for the parent

1. Record both sides and end mid-chunk on the new native. The final stretch appears exactly once. Optionally confirm the `read_audio_chunk` count equals the number of unique `sysaudio_*.wav` paths.
2. Stop the daemon, Stop & save, then restart the daemon and Retry. Exactly one note results, and the request log shows the same `recordingId` on both POSTs.
3. Fail the save, quit the app, relaunch and recover. The same `recordingId` and body are sent, and the result is one note, not a 409.
4. After a save, the card reads "queued" or "running" and then "done — N to-do cards". **Open note**, **Open board** and **Open triage session** land correctly. Reloading the window mid-triage brings the card back.
5. Force local-only mode with no local model. The card reads "skipped" and **Retry triage** is offered.

---

# Round 3 — source controls, per-app capture, modal focus (2026-10-07)

Same worktree, branch and model (claude-opus-5-5). Frontend only: no native, Rust, Swift, cargo, device or routing changes, UI control or commits. Scope: `lib/systemAudio.ts`, `hooks/useMeetingDictation*`, `components/voice/MeetingRecorder*`.

Context from the parent's live run: default output is External Headphones and default input is the C922 webcam mic. An acoustic test picks up the room, not the synthetic call. These changes let a recording skip the room mic and capture only the call's (or one app's) audio. They do not touch which devices macOS uses.

## Files (round 3)

| File | Change |
|---|---|
| `ui/command-center/src/lib/systemAudio.ts` | `SystemAudioApp`, `normalizeAppList`, `listSystemAudioApps()` (`null` = native cannot target an app). `startSystemAudioCapture(outDir, chunkSeconds, includeAppBundleId?)` sends the key only when set. |
| `ui/command-center/src/hooks/useMeetingDictation.ts` | `recordMic` (default **true**), `systemAudioApp` (default null = all), `activeSources` fixed at Start, `listSystemAudioApps`. System-only start path; at least one source required. `describeSources`, `RecordingSources`. |
| `ui/command-center/src/components/voice/MeetingRecorder.tsx` | "Record my microphone" checkbox. App select under "Also record the other participants". Description and live capture line name the actual sources. Start disabled without a source. Modal focus management. Pure helpers `pickerDescription`, `captureLine`, `focusables`. |
| `…/useMeetingDictation.lifecycle.test.tsx` | +8 source tests (44 total); one existing wording assertion updated |
| `…/components/voice/MeetingRecorder.test.tsx` | +4 tests (11 total) |
| `…/components/voice/MeetingRecorder.focus.test.tsx` | **new**: 4 real-render tests (only the network mocked) |
| `…/lib/systemAudio.test.ts` | +2 tests (6 total) |

## Checks (round 3, from `ui/command-center`)

| Command | Exit | Result |
|---|---|---|
| `npx vitest run src/hooks/useMeetingDictation src/components/voice/MeetingRecorder src/lib/systemAudio` | 0 | 6 files, 105/105 passed |
| `npx tsc --noEmit -p .` | 0 | no output |
| name-guard grep over the owned files (word-bounded) | — | no hits |

## Behaviour

**Mic is optional; the default is unchanged**
- "Record my microphone" is checked by default, so the old behaviour holds unless the user changes it.
- Unchecked, `getUserMedia` is **never** called; no mic is opened or prompted for.

**At least one source**
- With neither box checked, Start is disabled and an inline `role="alert"` says "Choose at least one source to record."
- The hook also refuses: it sets an error state and opens nothing.

**System audio only**
- Before anything else, `system_audio_available` is checked. If the helper is missing, the start fails with an explicit error ("…nothing to record. Turn on “Record my microphone”…") instead of a recording that captures nothing.
- If `start_system_audio` then fails, the start fails completely: timer cleared, seeded draft removed, target and sources cleared, recording identity retired, listeners removed.
- That error text never says the mic is still recording. With the mic on, a helper failure stays non-fatal and the text now reads "Your microphone is still being recorded."

**Labels say what is actually recording**
- The panel line comes from `activeSources`, fixed at Start, not from the picker toggles. Examples:
  - "Record Practice audio only — hearing Record Practice ✓"
  - "Your microphone + all system audio — mic: listening… · waiting for system audio…"
- A system-only recording never says "you" or "your voice". The transcript still labels far-side audio `**Others:**` (one mixed stream, never named people). With no mic chunks there are no `**You:**` lines.

**App selection**
- Shown only when "Also record the other participants" is checked.
- `list_system_audio_apps` is called each time the picker opens with system audio on. Results:
  - **Array:** a select whose default is "All system audio — everything this Mac plays", followed by "<App> only" entries, sorted by name. A previously chosen app that is no longer listed falls back to All.
  - **`null`** (command missing or errored, i.e. an older native): plain text "Captures all system audio. This build can't limit capture to one app." No select is shown, so there is no false claim of isolation.
- The chosen app is sent as `includeAppBundleId` only when one is chosen and system audio is on. With All selected, `start_system_audio` gets exactly its old arguments.

**Stable recordingId** is unchanged and still sent on system-only saves (tested).

**Modal focus**
- The dialog keeps `role="dialog"`, `aria-modal="true"` and `aria-label`; no roles were removed.
- It gains `aria-describedby` (the live source summary) and `tabIndex={-1}`, and the sources sit in a `role="group"`.
- On open, focus moves into the dialog: the first control, or the dialog itself.
- Tab and Shift+Tab wrap inside the dialog.
- On close (Escape, Cancel or Start), focus returns to whatever held it before, normally the Record button.
- Enter no longer starts the recording when it comes from the app `<select>`, and Start requires a source.

## Native spec this assumes (for the native worker; please confirm)

- **`list_system_audio_apps`:** `-> Vec<{ bundleId: String, name: String }>`, serialised camelCase. Entries with no or blank `bundleId` are dropped; a blank `name` displays as the bundle id. Any error, including "command not found", is read as "cannot target apps".
- **`start_system_audio`:** existing `out_dir` and `chunk_seconds`, plus optional `include_app_bundle_id: Option<String>`. The renderer sends it as `includeAppBundleId`, which Tauri maps to snake_case by default; if the native renames the argument (for example with `rename_all`), the key here must change.
- **Practice app:** bundle id `ai.permagent.record-practice-20261007` is used in tests as an example only. Nothing in code hard-codes it.
- **Unverified:** whether an older native rejects or ignores the extra key. The UI never sends it unless `list_system_audio_apps` exists, so this only matters for a native that has the list command but not the start argument.

## Tests added (round 3)

**Lifecycle**
- the mic is on by default;
- system-only never calls `getUserMedia`, labels the transcript `**Others:**` only, and still sends the recordingId;
- both sources off → error state, nothing opened, no draft;
- system-only with the helper unavailable → error, no `start_system_audio`, no mic, no draft;
- system-only with `start_system_audio` failing → error without "still being recorded", target and sources null, listeners gone, no draft, a late event files nothing;
- the chosen app reaches `start_system_audio` as `includeAppBundleId`, and the key is absent when All is selected;
- an app chosen while system audio is off is not sent;
- the app list is normalised, and a missing command returns `null`.

**Pure**
- `describeSources`, `captureLine` (no "you" when the mic is off), `pickerDescription`, `focusables`, `normalizeAppList`.

**Render** (`MeetingRecorder.focus.test.tsx`)
- focus lands inside the dialog with its roles intact;
- the mic checkbox defaults on, and unchecking it shows the alert and disables Start;
- Tab and Shift+Tab wrap;
- Escape closes and returns focus to the Record button.
- The first check fails against the pre-round-3 component, where focus stayed on the rail.

## Not verified / honest limits

- **macOS accessibility is not proven.** The parent saw the live Record modal absent from the AX tree even with the filter focused. These tests prove DOM focus semantics in jsdom only. They do not show that WebKit now exposes the portal dialog to macOS accessibility, and I am not claiming this focus work fixes that. Possible avenues for the native pass (unconfirmed):
  - the dialog is portalled to `document.body` behind `pushBrowserOverlay`;
  - how the Tauri webview publishes its AX subtree for `aria-modal` content.
- **No native run.** The app list, per-app capture and the `includeAppBundleId` argument are modelled from the brief; the native side is not built here.
- Full vitest suite not run, per the brief.
- The app select is a native `<select>` styled with tokens (the codebase has no shared Select component); it is not screenshot-checked at real size.
- `MEETING_CHUNK_SECONDS` alignment between a system-only recording's chunks and its elapsed timer is unchanged. With no mic there is nothing to interleave, so this is moot.

## Native checks for the parent

1. Open the picker. "Record my microphone" is checked. Unchecking both boxes disables Start and shows the alert.
2. With mic off, system on and Record Practice selected: no mic indicator lights. The native receives `include_app_bundle_id = ai.permagent.record-practice-20261007`. The note holds only `**Others:**` lines from the practice app's audio, not the room.
3. On a native without `list_system_audio_apps`, the picker shows "This build can't limit capture to one app" and no select.
4. With mic off, system on and the helper missing (or `start_system_audio` returning an error), the start fails with an explicit error and no recording.
   - A Screen Recording denial reaches the renderer as a `system_audio_error` event *after* a successful start. In system-only mode the recording then stays "Recording" with the permission error shown on the panel, but captures nothing.
   - That is visible, not silent, but it is not a failed start. If the parent wants it to fail the start or stop automatically, that is a small follow-up in the error-event handler; not done here.
5. AX: inspect the open dialog with the accessibility inspector. If it is still absent, focus is not the cause, and the portal/webview publication needs a native look.

---

# Round 4 — fail-closed app choice, system-only error after Start (2026-10-07)

This round supersedes two round-3 behaviours:
- A chosen app that disappeared used to fall back to All.
- Native check 4's sub-points: a system-only recording no longer stays "Recording" after a helper error.

## Checks

| Command (from `ui/command-center`) | Exit | Result |
|---|---|---|
| `npx vitest run src/hooks/useMeetingDictation src/components/voice/MeetingRecorder src/lib/systemAudio` | 0 | 6 files, 113/113 passed |
| `npx tsc --noEmit -p .` | 0 | no output |

## Files

- **`ui/command-center/src/hooks/useMeetingDictation.ts`**
  - New `appAvailability`, `unavailableAppMessage`.
  - Start re-lists apps and refuses an unavailable chosen app.
  - System-only shutdown on a helper error (`systemOnlyFailRef`); `draftKeptRef`.
  - Start does not flip to recording once that shutdown is under way.
- **`ui/command-center/src/components/voice/MeetingRecorder.tsx`**
  - The chosen app is kept.
  - It is shown as "<name> — not running" in the select, or as "can't be captured by this build" with a **Use all system audio** button.
  - `role="alert"` message; `aria-invalid`; Start disabled (`canStart`).
- **`…/useMeetingDictation.lifecycle.test.tsx`**: +6 tests (50 total); the IPC mock gains `list_system_audio_apps`.
- **`…/components/voice/MeetingRecorder.test.tsx`**: +2 tests (13 total).

## (1) Chosen app is never widened

**Picker**
- The list refresh no longer resets the choice.
- If the chosen app is missing from the list (or the native returns `null`), it stays selected and is marked unavailable. An alert explains it, and Start stays disabled until the user picks something else.
- While the list is still loading, Start is disabled.

**Hook**
- Start calls `list_system_audio_apps` again whenever an app is chosen.
- If the app is absent, or the native can't target apps, Start fails before opening anything: no mic, no `start_system_audio`, no draft. The choice is kept.
- A native failure on the exact-target start stays a failed start (fail closed).

## (2) Helper error after Start

**System only**
- A `system_audio_error` ends the recording:
  - **Stops:** the state goes recording → finishing → error, the timer is cleared, and the helper is stopped (any flushed tail is taken, deduped). Listeners are removed and the far queue drained.
  - **Draft:** what was captured, plus the notepad, is stashed as a draft and put on the recovered-recording card.
  - **Retires:** target, sources and recording identity are cleared.
- The error says system audio was the only source and that what was captured is kept (or that nothing was captured, in which case the empty draft is removed).
- Nothing is auto-saved.
- Dismissing the error panel does not delete the kept draft.

**Mic + system**
- The mic keeps recording.
- The error reads "System audio capture failed (…). Your microphone is still being recorded."
- The helper is not stopped by this path; the transcript still saves from the mic.

## Tests added

**Fail-closed app**
- chosen app not running → refused, choice retained, nothing opened;
- native without the list command → refused with the "can't capture a single app" message;
- exact-target `start_system_audio` failure → failed start, choice retained, no draft.

**Error after Start**
- system-only permission error → error state, helper and listeners stopped, no save, draft holds far parts and notes and is on the recovery card, Dismiss keeps it, Stop is a no-op;
- system-only error with nothing captured → "Nothing had been captured yet", no draft;
- mic + system error → still recording, accurate error, helper untouched, the mic transcript saves.

**Pure**
- `appAvailability` (available, unavailable, null, checking) and `unavailableAppMessage`.

## Not verified

- No native run.
- The picker's unavailable-app state is covered by the pure helpers and the hook refusal, not by a render test with a live app list (the render test runs outside Tauri, so no system-audio controls appear).
