# Shipping QA continuation — 2026-10-08

Resume base: `7f4046c3`, branch `world/production-20261006`, integration worktree `permagent-worktrees/world-production-20261006`. This continuation follows the user's renewed instruction to test Record thoroughly, walk every screen, and remove dependence on the developer's personal setup. **Not a release approval.** Current source, retained native review app, and running production daemon are different artifacts.

## Verified this continuation

- Full frontend suite: **486 files, 3,923 tests passed**, zero skipped/failed, one isolated fork at a time, 324 seconds. Receipt: [isolated suite](SHIPPING_FRONTEND_ISOLATED_2026-10-08.json). The earlier single-process run failed from shared test globals; it remains recorded separately, with investigation in [test isolation audit](SHIPPING_TEST_ISOLATION_AUDIT_2026-10-08.md). Do not sum overlapping focused counts into a new total.
- Record regression coverage includes readiness before opening capture, suspended AudioContext resume failure, microphone/system gaps, no-transcribed-speech notice, Cancel Enter behavior, save-retry ownership, partial native-stop failure and final-tail deduplication. Focused Record receipt: [readiness and lifecycle](../record-evidence/record-readiness-audit-20261008.json).
- Five standalone Rust tests execute the production lifecycle module for exited children, graceful final flush, exit failure, reader panic/drain and timeout. [Receipt](../record-evidence/native-lifecycle-standalone-20261008.json). This is not a full Tauri compilation/IPC acceptance.
- Foreground synthetic WebRTC call → exact-Chrome ScreenCaptureKit helper now captures nonzero real PCM. Full 45-second chunk plus 30.18-second post-stop tail, no helper/protocol errors, clean exit and reader completion. Independent verification recomputed WAV hashes and metrics. [Capture report](../record-evidence/FOREGROUND_CAPTURE_2026-10-08.md). This supersedes incoming-silence results only for this controlled run; intelligibility, tail semantics and integrated Record are separate gates.
- Read-only Chrome-family HAL and default-output probes identified active output during native-foreground playback. Earlier browser-controlled background runs had no active Chrome output. No device changes, permission expansion, audio taps or browser security changes were performed. A Homebrew Python `plistlib` library error affected the first foreground metadata attempt; the successful retry used system Python. Failed evidence is retained, not counted as a measurement.

## Current-source Computer Use observations

Context: Chrome source preview `127.0.0.1:5301/ui/` connected to the existing production daemon3001. Read-only business views and reversible UI drafts/preferences only; these observations are not a fresh native install. Controls were rediscovered after transitions.

- Record picker explains browser microphone-only capture, disables Start with no source and reports missing readiness support from the older backend. Retry remains an honest error; no false Ready. No capture started through this picker.
- Home Calendar expands/collapses; Day/Week/Month arrow/Home navigation changes views without silently advancing dates. Next day shows an unchecked-date explanation with today-only-source limits. Today/Month restored; Escape collapse returns focus to Expand Calendar.
- World Lantern night persists across Home→World; System restored. Close Desk returns focus to Agent desk. Reduced motion preference was toggled and restored; this was not a measured frame-animation or static-portrait acceptance.
- Automate recurring→one-time draft has explicit local-time guidance without an ineffective timezone control. Cancel leaves no job.
- A separately unpaired localhost browser origin shows pairing guidance and Retry. This proves unpaired browser behavior, not fresh native onboarding.
- People: Add Person empty draft disables Add; Cancel returns to its trigger. Nonsense search produces “No people match that search.” Clearing restores the list. Graph renders and exposes a filter and Key control. No people were created or changed.
- Finance: screen exposes lots, picks, household, watchlist and notes; Open→All filter changes selection and includes the closed lot. No holding/trade/watchlist edits, scanner execution, or finance enablement. Attempt to restore Open coincided with the user changing Chrome windows; subsequent dedicated-tab observation confirmed Open selected and All unselected.
- At approximately15:04UTC native Chrome switched to the user's unrelated browsing window. Native actions stopped. Binding the dedicated QA tab through the browser API timed out on `Emulation.setFocusEmulationEnabled`; unrelated window was left alone. Resetting the CUA binding recovered dedicated-tab control without using the personal window. No new capture may run without re-verifying task-only Chrome audio scope.

## Shipping fixes and open gates

[First-user audit](FIRST_USER_SHIP_AUDIT_2026-10-08.md) records source fixes for native build URL contamination and truthful model provisioning/readiness. New backend GET status validates model availability without claiming inference. Existing daemon lacks this route; current frontend intentionally blocks capture against it. Current backend/native changes still require authentic builds and HTTP/IPC tests. Retained native r3 predates them.

Fresh first-user Brain initialization was found to depend on an existing ontology. A production-only bootstrap now creates an empty product ontology exclusively in a newly created Brain directory, preserving all existing data; six filesystem tests pass. The added actual AppState integration test is not yet compiled/run. See [bootstrap report](FRESH_BRAIN_BOOTSTRAP_2026-10-08.md). Do not treat the previously seeded Record QA root as first-user evidence.

The complete [screen checklist](SHIPPING_SCREEN_CHECKLIST_2026-10-08.md) remains open. Highest-priority remaining acceptance:

1. Captured WAV → local ASR, including negation/cancelled purchase and final commitment specifically in the stop-flushed tail.
2. Current native Record UI → capture/ASR → project-bound saved note → Orchestrator triage → authoritative Brain catalogue → correct unexecuted Kanban commitments, stable on retries.
3. Fresh empty data root, no credentials/model/service, setup/Decide later/restart and Brain availability; then signed downloaded native installation, permission deny/retry, resource relocation and no developer PATH.
4. Remaining screen flows and meaningful empty/error/recovery states, plus native-only Build, voice and Calendar paths.

## Resource and artifact boundaries

Current free disk around22GiB. Existing safety guards remain25GiB to start heavy builds/isolated daemon and20GiB stop floor. No heavy backend/native build or isolated daemon launch occurred this continuation. Tests used bounded memory/one fork. Six unused owned pctx build-script executables were removed (~212MiB), with [cleanup receipt](SHIPPING_QA_BUILD_CACHE_CLEANUP_2026-10-08.json). A later owned-target inventory found only0.304GiB additional regenerable scripts; they were retained because deleting them would not resolve the resource block and would increase rebuild cost.

Production daemon3001, installed desktop app, Ollama11434, other-lane previews and unrelated work are preserved. Capture audio/receipts are private under the owned shared target, outside the fixture web server. Do not publish them or substitute source fixture audio for actual capture.

## Final continuation observations and checkpoint boundary

After dedicated-tab recovery, Record Cancel was activated with Enter with the disposable QA project selected. Picker closed and focus returned to Record meeting; no capture started. The existing backend readiness error still disabled Start, so enabled-Start Cancel behavior remains supported by regression tests rather than this UI run.

History: absent token `zzqvnotarealsessionkpx` settled to zero counts and a kind-specific “No agent runs match” message. Search cleared and Conversations restored. Downloads exposed its explicit Brain/Project/Post routing controls without routing any existing file; Activity displayed runtime events; Spend exposed labeled optional budget fields without edits. Settings navigation exposed all13 sections and Persona fields; every pane's deeper flow is not yet covered.

Brain source and entity filters were exercised and restored. The source filter initially showed zero while loading, then returned populated briefings; the initial loading snapshot is not an empty-data result. Search terms are OR substring matched in chat history and also used for semantic recall. Source review therefore did not establish a query-ignored defect from apparently unrelated excerpts. Three frontend files now correct “matching memories” to explained search results, preserve conversation provenance, clear stale results while the next query loads, and format future timestamps explicitly. Worker reports23 focused tests across2files passed. These edits occurred **after** the3,923-test full-suite checkpoint; final full suite, TypeScript check and CUA recheck remain pending because disk fell below the floor.

`dictation.rs` also now gates existing provisioning helpers correctly for builds without local inference; the disabled route returns unavailable without provisioning. A three-handler regression test was added but not run under a full no-default-features backend build. Agent self-knowledge now explains readiness, browser/native source limits, transcription-delay warnings and recoverable audio gaps.

Final free disk fell to approximately18GiB. The owned Vite5301 process group60737 was identity-checked and stopped with SIGTERM; supervisor88326 exited0. Fixture5489 expired normally. Isolated3002/3003 and5302 remained stopped. All workers, capture and tests have finished; no new inference ran. Production3001 was preserved. Browser tabs may display cached source despite stopped previews.

The owned shared target and main-repo cache inventories found no safe multi-GiB cleanup set. See [main-repo inventory](MAIN_REPO_CACHE_INVENTORY_2026-10-08.md). Do not weaken the25GiB start/20GiB stop guards or delete source/evidence/active dependencies to resume. With headroom restored, first transcribe the retained successful capture, then compile/test current backend/native and run the genuinely fresh-user/integrated Record gates. Latest source is a review checkpoint, not a validated shipping artifact.

## Resume after cleanup — October 8, 15:34–15:40 UTC

Resumed from `dee84a31` after the user's disk-pressure/resume instruction. Initial free disk was approximately22GiB (95% data-volume usage), below the25GiB heavy-build/isolated-runtime start threshold. Only TypeScript, a single-fork frontend suite and the owned loopback source preview were attempted, each with the20GiB stop floor. No cleanup, native build or inference was performed in this continuation.

The pending TypeScript gate found `BrainList.tsx:639` still calling the removed `formatDate` helper. CUA independently observed the Brain error boundary on the current source preview. Updated the entity-row call to `formatMemoryDate`; TypeScript then passed. Brain recovered through Try again and LIST, showing80entities/5,089memories, a dated entity row (`updated Aug 25`), and explicit future memory dates. A nonsense search settled to0search results with the new any-word/related-meaning explanation and useful empty-result guidance. This is browser evidence against production daemon3001, not native or fresh-root acceptance.

Disk then fell below20GiB. The supervised frontend suite received SIGTERM after32seconds at19.869GiB, and the preview exited. The subsequent topic search displayed Failed to fetch after preview shutdown; it is not a successful provenance check or an established backend defect. The interrupted suite provides no full-suite pass. Initial failure, successful TypeScript recheck, interrupted suite, timings and minimum disk are preserved in [resume validation](SHIPPING_RESUME_VALIDATION_2026-10-08.json). `git diff --check` passed.

Final disk observation: approximately19.0GiB available,96% data-volume usage,5.79GiB swap used. Owned5301 and test processes are stopped; production3001 remains listening. The25GiB startup guard and20GiB stop floor remain unchanged. The formatter fix is ready; the complete frontend rerun, remaining Brain search provenance check, actual captured-audio ASR, current backend/native tests and fresh-user/integrated Record acceptance remain pending. No release readiness is claimed.
