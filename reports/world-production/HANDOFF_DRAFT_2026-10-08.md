# Handoff draft — October 8, 2026

Superseded by [the final October 8 handoff](HANDOFF_2026-10-08.md). The active-build statements below are historical draft state.

**Draft only. Work remains active; do not treat this as completion or release approval.** Update the active build and runtime outcomes before turning this into the final handoff. This document separates verified browser behavior, source/test evidence and native acceptance gaps.

## Workspace and ownership

- Integration branch: `world/production-20261006`; checkout `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006`.
- HEAD remains `b55bf214feeb41bd5c9708cfafe628f98c491608`. The current session's implementation and reports are working-tree changes; the original clean-tip description no longer describes the checkout. Preserve them and inspect the diff before committing.
- Coordinator owns Computer Use, model calls, runtime processes and heavy builds. Claude supplied Record/Home work; Cursor supplied World work; source audit/navigation/testing lane supplied scoped fixes and evidence. Frontend source is currently frozen for stable CUA.
- Sole active heavy build at this draft: coordinator session **13148**, [daemon build log](record-backend-tests-20261008-000417.log). **Outcome pending.** Do not start another heavy compiler until the coordinator confirms completion.
- Preserve installed `/Applications/Permagent.app`, shared daemon 3001, unrelated user applications and benchmark targets. Use the isolated Record runtime for synthetic writes.

## User objectives and current boundaries

| Objective | Verified | Still open |
| --- | --- | --- |
| Detailed, realistic World using FAL | Integrated imagery received browser review in the daytime archive and observatory garden; all six Places navigate to expected collaborators; dock/rail overlap fixed; Walk/W/D/Escape inspected. [FAL report](FAL_VISTA_REPORT_2026-10-07.md), [browser evidence](CUA_BROWSER_2026-10-07.md). | Final native lighting/scenery, night/reduced-motion, complete walking/collision routes and every desk/lesson branch. |
| Test every app flow | Every main browser area was visited with bounded subflow evidence; flow matrix remains deliberately open. Home expanded cards retain local state; Build keeps 64 runs bounded; labels and browser fallbacks were rechecked. | Native coverage, deeper actions and all matrix requirements. Do not convert visited views into all-flows passes. |
| Record practice meetings, catalogue transcripts and create relevant todos | Direct ASR fixtures preserve tails; three project-bound HTTP scenarios pass 120 checks, including two commitments per project, Brain delivery, provenance, local routing and retry idempotency. | Actual selected-source capture returned silent PCM; no complete capture → ASR → triage → cards pass. |
| App knowledge and navigation | Current self-knowledge map plus allowlisted Home card focus, project lenses, World Places, History and Record-picker navigation; persistent generated-card source affordance. | Native delivery checks. Navigation acknowledgement describes a request, not an observed action receipt. Record navigation never starts capture. |
| Haiku 5.5 | Exact model catalogue/format support and additive live picker entry; defaults unchanged. | No Haiku inference acceptance claimed. |

## Latest verified checks

- Browser 5301 Settings: nine notification switches, Remote access and search/data-source switches have names. Missing normal Vite `/integrations`, `/gateway`, `/tunnel` proxies were added; Vite auto-reloaded and CUA observed Gmail **Not connected** and Remote tunnel **Off**. No credentials/toggles were changed for the inspection.
- World Open Shared Chat connected, showing an existing conversation containing “pong” and the dock composer. [Stable screenshot](world-shared-chat-browser-20261008.png). No new send/response round trip is claimed.
- Isolated browser 5302: normal pairing and fresh setup completed via Decide later, RecordQA/Balanced, folder/Extras skips and Start. Existing local model configuration remained unchanged. The delayed provider list now has a source loading indicator; the Review ellipsis at Start remains a follow-up observation.
- Frontend full suite: **476 files / 3,809 tests** passed before later scoped edits. Subsequent suites overlap; do not add them into a new total. Latest Settings/Toggle/proxy **36/36**, wizard **9/9**, and TypeScript passed. Shared Toggle now recovers both fulfilled and rejected asynchronous writes under StrictMode.
- Backend: fresh meeting tests **40/40**, targeted tests **116/116** (one ignored and one explicitly excluded global-config-mutating test), and latest Brain note-association retry regression **1/1**. [Targeted receipt](FRESH_BINARY_TARGETED_TESTS_2026-10-08.json), [latest Brain result](record-backend-tests-20261007-235901.json). The latter took 300.76 seconds including compile, minimum free 21.965 GiB.
- Earlier daemon build passed in 511.92 seconds. The latest rebuild after the Brain correction is still pending. Daemon route-unit compilation was deliberately stopped before projected disk pressure; it has no pass result.
- Native Rust checks **7/7** cover system audio/runtime targeting. They do not prove audio arrives in a live capture.

## Record evidence and native artifact

[Original](../record-evidence/isolated-meeting-http-20261008-024958-original-f9995b62.json), [paraphrase A](../record-evidence/isolated-meeting-http-20261008-025035-paraphrase-a-9ce86708.json) and [paraphrase B](../record-evidence/isolated-meeting-http-20261008-025302-paraphrase-b-5fd0216b.json) each pass 40 HTTP checks: six projects and twelve commitments total. These submit synthetic transcripts. [ASR evidence](../record-evidence/isolated-whisper-20261008-015232/evidence.json) independently passes eight semantic checks across three WAVs.

Current stage: `.shared-target/world-production-20261006/staged/Permagent Record Review (record-r2).app` under `/Users/j/Documents/dev/permagent-worktrees/`. It targets external daemon 3002 and the isolated `record-qa-runtime`; package verified 614 assets. [Package receipt](native-record-package-verification-record-r2.json) contains hashes. It predates late wizard/Toggle/proxy edits. No installed-app replacement occurred.

Live Chrome-scoped capture reported granted preflight, verified source, READY and clean exit, yet both 45-second and 30.22-second WAVs had peak/RMS zero. The latter flushed after SIGTERM. Simultaneous WebRTC showed 2,733 received audio packets, nonzero energy and 252 decoded frames. No ASR was attempted on silence. Private receipt: isolated runtime `capture-evidence/chrome-272dc6f4655f4ce3925981ac4b38802e/capture.json`; do not publish raw private captures or authentication material. Native CUA previously returned `cgWindowNotFound`; that is not evidence of a crash.

## Resume surfaces and resource rules

- Browser 5301 `/ui/`: normal preview against production daemon 3001; only authorized reversible inspection there.
- Browser 5302 `/ui/`: isolated review. [Browser route and pairing procedure](../record-evidence/BROWSER_REVIEW_ROUTE.md); `isolated_browser.py` uses a pinned trusted Node binary and an owned loopback Vite group. Normal claim pairing keeps the daemon bearer out of browser handoff logs.
- Practice fixture: loopback 5489, [fixture instructions](../record-evidence/README.md). Real WebRTC and explicit **Direct audio test — NOT a call** are distinct modes. Captions are ground truth, not observed ASR. Parent/operator owns playback and capture.
- Guarded builds require 25 GiB startup and stop at 20 GiB; inference also creates disk/swap pressure. Free space varies with external activity. Do not weaken guards or clean global caches to continue.
- Coordinator removed only audited obsolete artifacts and the stopped World/r1 stage bundles. Current r2, dependency roots, runtime evidence and benchmark targets are retained. The latest cancelled-route audit identifies only **68 files / 0.274 GiB**, not several GiB. [Exact manifest](ARTIFACT_RECHECK_ROUTE_VARIANTS_2026-10-08.json). Revalidate hashes/process state before any coordinator-approved removal; this audit is not deletion authorization.

## Next acceptance gates

1. Resolve the active daemon build result and record its binary hash. If it succeeds, inspect the corrected Brain retry behavior through the isolated runtime without implying old HTTP evidence used that newer binary.
2. Diagnose zero-PCM capture using supported scoped capture/permission paths. Then prove actual practice-call audio, final tail, project note, Orchestrator session, catalogue and stable unexecuted Kanban cards.
3. Refresh the native stage for late frontend changes when resources permit; verify Home state retention, World rendering/navigation, persistent Source meeting links and permission states through native CUA.
4. Continue the unchecked [flow matrix](FLOW_MATRIX_2026-10-07.md), especially voice, wizard edge cases, native calendar ranges and deeper actions. Preserve partial/failed results rather than claiming every flow passed.
5. Before final delivery, reconcile the working-tree diff and current receipts, update this draft's active statuses, and write a concise final handoff with remaining gaps. No release or overall Record completion is currently established.

[Current checkpoint](CURRENT_CHECKPOINT_2026-10-07.md) remains the detailed live status; [browser report](CUA_BROWSER_2026-10-07.md) records exact CUA boundaries.
