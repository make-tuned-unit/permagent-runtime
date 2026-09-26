# QA walk resume recovery — 2026-09-26

Recovered Claude session `6f676076-ccbb-467a-8de1-cc7282c8f988` (QA walk resume).

## Preserved and integrated

Seven formerly uncommitted fix groups are now committed and merged into `dagj/qa-int3`:

| Group | Preserved commit |
|---|---|
| Cache/provider message ordering | f0a8173d |
| Progress breaker/drain | 2b47b60a |
| Inbox daemon | ffe95aff |
| Local context-window routing | d9aefe79 |
| Inbox UI | edc4e8d9 |
| SWE benchmark | d8fa3725 |
| Reasoning/preamble history | 07f2d6ea |

Integration merge tip before follow-up fixes: `5de8680a`.

The workflow's completed status was not evidence of complete verification: five groups lost final verification/repair stages to the usage limit. Rust changes are not accepted until the combined gate runs.

Three inbox merge conflicts were resolved preserving both budget resets and human zero-progress resets, the NoProgress summary, and both new test sets.

## Follow-up fixes

- Command-approval resume clears the human-authorized zero-progress breaker, preventing the next dispatch from immediately parking the goal again. Added an end-to-end SQLite decision-effect test.
- Eval regrade leaves judge infrastructure errors, timeouts, and missing oracle exit codes errored. Added a file-backed regrade regression test.
- Real Chrome accessibility inspection found DetailModal's close button unnamed. Added `aria-label="Close"`; Chrome now exposes it as Close.
- Formatted the merged Rust changes.

## Actual validation

- TypeScript: passed.
- Full frontend Vitest before the close-label follow-up: **3428 / 3428**, 432 files.
- Frontend production build before the close-label follow-up: passed (existing large-chunk warning).
- TypeScript after close-label follow-up: passed.
- Focused inbox/modal Vitest after close-label follow-up: **182 / 182**, 17 files.
- Rust formatting and git diff whitespace check: passed.
- Real Chrome: isolated mock inbox at `http://127.0.0.1:5299/`; full-size sheet, choice and parked-goal views, Retry with notes input, and named Close control inspected. No live approvals or messages were submitted. Preview imports the integrated real components and built CSS; it is outside the repository at `/private/tmp/qa-resume-preview`.
- Rust compilation/tests/clippy: **NOT RUN for these merged fixes**. Build guard exited 10 with only 4 GiB free, below its 10 GiB minimum. No guard override used.
- Packaged-app acceptance/release and SWE-arm-30 baseline: **pending**.

## Coordination and next work

The other recovery thread is `01a0dfd0-0270-71d0-9a07-e918a043b6da`, recovering Claude's UX overhaul in `/private/tmp/permagent-qa-ux-resume`, with warm target `/private/tmp/qa-ux-target` (~22 GB). This thread is `01a0dfcf-0135-7c40-9f97-42a444e4260d`.

Jesse explicitly requested coordination. A supported `codex queue` message was accepted as `01a0dff5-7a43-72b1-9b00-53eb6ad8b1a6`. Detailed proposal: `/private/tmp/qa-ux-recovery/COORDINATION_FROM_QA_WALK.md`. No handoff acknowledgement yet at this checkpoint. A direct app-server steering attempt timed out during initialize and did not send a message.

1. Obtain the UX thread's completed commit and warm-target handoff. Do not run simultaneous builds or delete its artifacts.
2. Merge its final UX tip into `dagj/qa-int3`, reviewing overlaps in provider cache handling and orchestrator code.
3. Recover enough disk; use a single warm build folder under the build guard. The original project prune script already ran safely during a no-cargo gap (original target now ~10 GB).
4. Run the combined Rust gate (lib/bins and test-profile clippy for permagent, daemon, CLI, eval; lib tests; reasoning agent integration tests; daemon tests through `scripts/test-daemon.sh --lib` for dylib signing). Validate new command-approval and eval regrade tests first. Run frontend checks again after UX merge.
5. One release, provenance check, installed-app acceptance, then the previously approved benchmark baseline (`dschat`, then `dschat+ornith`).

The main checkout's unrelated iOS/untracked work is untouched. No release was installed and no benchmark spend incurred in this recovery pass.
