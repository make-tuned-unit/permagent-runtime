# Resume source/build audit — 2026-10-07

Inspected production HEAD `b55bf214feeb41bd5c9708cfafe628f98c491608`. No tests, builds, cargo, native launch or UI control were run by the initial audit. `FLOW_MATRIX_2026-10-07.md` contains the durable navigation/subflow inventory and explicitly separates historical evidence from current pending verification.

## Immediate findings

- **Missing runner:** `/tmp/permagent-native-review-build.py` no longer exists. Do not run the CHECKPOINT shortcut expecting it to work. Durable underlying command is in `NATIVE_STAGING_PLAN.md`.
- Native review is a different shell identity, **not a different backend**. It shares localhost:3001 and `~/.permagent`; fresh wizard testing requires a separate disposable runtime/config arrangement. Opening a second bundle alone does not provide isolation.
- Voice idle mic click immediately calls `getUserMedia` (`hooks/useVoice.ts:1475`); it cannot be counted as a non-mic connection check. Restrict live non-mic pass to idle controls/settings/Record modal cancellation; isolated state harness evidence must be labeled separately.
- Settings navigation has 13 current panes. Workspaces are daemon-defined. World compact and Previous campus are distinct branches. Evidence contains three fully local fictional demonstrations suitable for complete interaction testing.
- At audit `df -g /System/Volumes/Data` showed 32 GiB available. This is a point-in-time observation, not build-slot clearance. Handoff says benchmark peer requires >20 GiB and one heavy build at a time.

## Integration gate script audit

`/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh` accepts `fmt`, `check`, `clippy`, `clippytest`, `testlib`, `testsrv`, `testcli`, `named`, `ui`. Default WT is this production lane; target is always `.shared-target/world-production-20261006`, jobs=2, outputs `wq-reports/gate`.

After coordinating the heavy slot, intended sequence is:

```sh
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh fmt
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh check
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh clippy
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh clippytest
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh testlib
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh testsrv
/Users/j/Documents/dev/permagent-worktrees/wq-gate.sh testcli
```

Review these known script limitations before using it for evidence:

1. Its low-disk watcher calls **global** `pkill -f 'cargo (test|clippy|fmt|check)'` and `pkill rustc`, potentially killing unrelated work. Change/reconstruct the monitor to terminate only the owned process group before concurrent use; do not lower the floor to avoid coordination.
2. The 18 GiB monitor begins after launch; repository build guard does startup refusal, but with its own threshold. The peer's >20 GiB need and native reserve may be stricter.
3. `ui` does not enable `pipefail`: vitest and Vite are piped into `tail`, then the whole chain into `tee`. Their failures can be masked. Run uncropped commands with checked exits (or a corrected wrapper) rather than treating this branch's exit 0 as trustworthy.
4. `named` passes many trailing positional filters to a single libtest command; libtest normally accepts one TESTNAME. Use separate filtered runs or the whole lib suite. Do not assume this branch covers every listed name.
5. Three main-crate checks do **not** compile Tauri `ui/desktop/src-tauri`; wizard/calendar native bridge paths need the native shell build too.
6. `fmt --all -- --check` is read-only. Gate sources may report unrelated baseline failures; do not silently rewrite unrelated code.

## Exact native build command and prerequisites

First coordinate all heavy work, check memory/disk, exact running processes and current source. Preserve installed baseline/rollbacks and other lane targets. Ensure ignored `ui/desktop/src-tauri/binaries/*-aarch64-apple-darwin` and `resources/` inputs exist and are hash-matched to baseline ledger; target-root daemon/CLI may need restoring from those prerequisites if previously reclaimed. Do not borrow another lane's target.

From `/Users/j/Documents/dev/permagent-worktrees/world-production-20261006/ui/desktop`, the durable command is:

```sh
env WORKTREES_ROOT=/Users/j/Documents/dev/permagent-worktrees \
  CARGO_TARGET_DIR=/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006 \
  CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 \
  /Users/j/Documents/dev/permagent-worktrees/world-production-20261006/scripts/build-guard.sh --min-free-gb 18 -- \
  node ../command-center/node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle \
    --config /Users/j/Documents/dev/permagent-worktrees/world-production-20261006/reports/world-production/native-review-config.json \
    -- --locked -j 1
```

The 18 GiB startup threshold above conservatively preserves the historical 8 GiB incremental reserve + 10 GiB floor. A supervising wrapper is still needed for a bounded 30-minute timeout and continuous floor, terminating **only this command's process group**. `build-guard.sh` provides startup space/target locking, not continuous monitoring. PATH must resolve the repo's intended Rust/Node toolchain (`/Users/j/Documents/dev/permagent-runtime/bin` was gate precedent). If offline archives are missing after cleanup, report the actual missing dependency; the old cache-recovery claim is historical.

Keep inherited `beforeBuildCommand`: Tauri must build current frontend with native `/assets/` base, not browser `/ui/assets/`. Review config disables updater endpoints and updater artifacts, preserves existing source security/window settings and embeds unique `ai.permagent.world-review-20261006` identity. Do not use stale compiled dist after UI edits.

After build **completes successfully**, from worktree root:

```sh
node reports/world-production/verify_native_review.mjs
```

Then quit only the existing review app through Computer Use, confirm its exact executable is absent, and run:

```sh
python3 reports/world-production/stage_native_review.py --refresh
```

Use `--refresh` only for an existing stage with matching ledger; omit it only for genuinely new staging. The script preserves the prior shell/signature metadata, verifies embedding ledger vs raw binary, copies independently, signs, verifies ARM64/deep strict signature and hashes installed baseline unchanged. A running app or stale/mismatched ledger intentionally blocks refresh.

Launch the exact path using Computer Use:

`/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/staged/Permagent World Review.app`

Record raw/staged hashes, actual app identity and screenshots. Native smoke: arrival/overview, day/night/System, full bridge walk both directions, agent body focus, desk all tabs, Escape, reduced motion, hidden/regain, Settings keyboard scrolling and chat/Record navigation. Shell build success does not verify these behaviors. Replacing `/Applications/Permagent.app` is a separate delivery step with release provenance checks; this audit does not perform it.

## Parent CUA findings assigned for follow-up

- **QA-WD1:** Current visible Chrome :5301 World Agent desk → Meet places the eyebrow “Your sovereign orchestrator” in a narrow side column with excessive spaced wrapping. Inspect portrait grid sizing and correct only desk/portrait UI; do not alter scenery lane files.
- **QA-NAV1:** Current visible foreground Chrome :5301 Home click hovers but stays on World. Investigate shared Button StrictMode lifecycle; prior handoff mentioned its `live` ref bug. Parent will verify after HMR.

## Scoped fixes after parent extended the assignment

Parent authorized source changes for QA-WD1 and the shared Button lifecycle, plus focused testing.

- QA-WD1 cause: `.forum-agent-portrait` laid a nonshrinking 220×260 portrait beside text inside a 370px desk with panel/card padding. The remaining text column was too narrow for the role eyebrow. Changed the card to vertical flex, centered the portrait, gave identity full available width, bounded portrait width and reduced eyebrow tracking. Files: `ui/command-center/src/components/world/forum/forum.css`. Parent owns current CUA visual acceptance.
- Confirmed Button StrictMode defect: effect cleanup set `live.current=false`, but replayed setup never restored it, so async action settlement/pending-floor timers stopped updating the still-mounted button. Restored liveness at setup. Two meaningful regression tests exercise async success/reclick and externally-driven pending floor under StrictMode. Files: `ui/command-center/src/components/common/Button.tsx`, `Button.test.tsx`.
- Validation: `node node_modules/vitest/vitest.mjs run src/components/common/Button.test.tsx --maxWorkers=1 --minWorkers=1` from command-center: **22/22 pass**, 1.55s reported suite duration. Existing act/auth diagnostic warnings remain. `git diff --check` passed.
- QA-NAV1 attribution remains open: Sidebar's handler invokes `switchWorkspace` synchronously, and the store immediately updates `activeWorkspaceId` before persisting remotely. Button's `live` ref only gates async feedback. The lifecycle fix alone is not proof that Home navigation is corrected; parent must recheck the actual visible UI and investigate separately if it persists.
- No cargo/build, app launch, app UI automation, production installation, other-worktree writes or source scenery changes were performed by this worker.

## Native prerequisite audit and guarded runner (not executed)

Parent requested a stricter runner for the benchmark peer: start only at **25 GiB**, abort below **20 GiB**, maximum **30 minutes**. `reports/world-production/run_guarded_native.py` now implements these limits, 2-second disk sampling, one compile job/offline/no dev symbols/no incremental, inherited native frontend build, unique timestamped log/JSON summary, and TERM then KILL of only its `start_new_session=True` child process group. It never stages, launches or installs. It validates exact lane, native input presence, baseline sidecar/dylib hashes, review identity and disabled updater config before starting. It does not claim to negotiate the heavy slot; coordinator must do that first.

Exact next invocation, after the slot is free and source is ready:

```sh
python3 reports/world-production/run_guarded_native.py r29
```

Choose a fresh revision tag matching actual iteration (r29 is an example; no build executed). Subsequent embedding verification, CUA quit, staging refresh and CUA launch remain separate steps above. Runner source was parsed with `ast.parse` successfully; the runner itself was **not executed**, per parent instruction. Runtime process termination behavior is source-reviewed, not execution-tested.

Read-only prerequisite hash audit found:

- Both native sidecar inputs (`permagentd-aarch64-apple-darwin`, `permagent-aarch64-apple-darwin`) and both dylibs exist and match **both** `native-baseline-copy-ledger.json` and installed bundle bytes.
- `resources/mcp-runtime`: 4,614 files, 136,279,705 bytes, no per-file installed-baseline mismatches.
- `resources/whisper`: 1 file, 78,067,328 bytes, matches installed baseline.
- Audiocap input exists, matches installed Resources counterpart, SHA256 `4f5f0dabcd32778d3971edb0dba590eb5747595b0014d5a8d5029d2272a48a9e`.
- Tauri JS CLI, native Cargo.lock and Entitlements.plist exist.
- Target-root `debug/permagent` and `debug/permagentd` are **absent** from previous reclamation. This runner does not restore them. Existing staged review retains baseline copies. If direct-debug launch/bundling or a build step requires those target-root files, restore exact copies from the verified sidecar inputs; a cached Cargo build may not re-run the copy step. Staging helper copies the installed complete bundle rather than launching the raw debug output.
- Offline Cargo archive completeness is **not** re-proven without dependency resolution. No cargo was run.

## Fresh wizard isolation audit — second source pass

There is **no safe existing runtime invocation for the current staged native artifact** to point at a disposable daemon while the real :3001 daemon remains running:

- `main.tsx` recognizes `?view=chat`, `?view=pane`, and two DEV geometry harnesses. There is no wizard URL/CLI switch. `BootScreen` classifies the authenticated backend `/config` response; wizard appears only if `wizard_complete` is absent/false (`lib/startupConfig.ts`).
- Native `daemon.rs:42` detects `127.0.0.1:3001`; sidecar spawn at lines 291–311 builds paths beneath real `HOME/.permagent`, supplies explicit `--host 127.0.0.1 --port 3001`, and sets real config/spectral paths. No review-root/port runtime option exists. Heartbeat also writes real HOME/.permagent/logs.
- Native `get_daemon_token` at lines 511–519 reads real `HOME/.permagent/secrets/daemon_token.json`, ignoring `PERMAGENT_PATH_ROOT`.
- Frontend `lib/api.ts:16` can change base URL using **build-time** `VITE_DAEMON_URL` or `VITE_API_BASE_URL`, but current native CSP only permits the usual :3001 HTTP/WS endpoints. A shell rebuilt with a new URL alone still uses the wrong token and supervisor.
- The standalone daemon supports `permagentd agent --host 127.0.0.1 --port PORT`; `PERMAGENT_PATH_ROOT` redirects shared Paths config/data/state and `PERMAGENT_DISABLE_KEYRING` avoids shared credential storage. These daemon capabilities do not solve the native app's separately hardcoded paths or guarantee isolation of every background subsystem. No isolated daemon was launched.

The robust actual-native route with **this unchanged artifact** is a disposable macOS VM with its own HOME, port namespace and keychain. Copy the staged review bundle into that VM and launch it through normal app UI; let its own bundled daemon initialize an empty profile, then run the six-step wizard. A second macOS login on the same host is not enough while :3001 is live because loopback ports are shared across users. No HOME override, production wizard flag reset, token copy, daemon stop or native interception is proposed.

To enable same-host native QA later requires a deliberate review-only runtime profile contract spanning native daemon address/lifecycle, token and heartbeat/config paths, frontend base URL and CSP. That is a separate implementation, not a safe last-minute launch flag. An isolated browser harness can exercise the real React wizard but must remain labeled browser/harness evidence, not a fresh native install.

## Integrated calendar review and fixes

- Confirmed source bug: `fetchDaemonRange` can only read today; ranges excluding today return `[]`. The hook honestly reports source `daemon`, but CalendarCard previously treated that empty partial result as “No events this week/month” and “Nothing coming up in the next 7 days,” including when fullAccess EventKit failed on an older native shell.
- Fixed CalendarCard to disclose today-only coverage whenever daemon fallback is used, never claim an empty queried week/month unless EventKit actually answered, and restrict the seven-day Next up empty claim to EventKit. An exhausted daemon feed now says later dates are unavailable. Also fixes the EventKit Next up case where only already-finished events exist: empty-looking future strip no longer claims a missing connection.
- Saved 5-column layout is preserved. Rather than silently resizing persisted user layout or removing view controls, full Week now has a 560px minimum readable content width in a horizontally scrollable wrapper, with a scroll hint in narrow cards. Header and columns move together. Day/Month controls remain available. Parent should visually verify the exact saved layout after native build.
- Only `CalendarCard.tsx` and `CalendarCard.test.tsx` changed in this pass. Added regression for older-shell empty daemon fallback → next week/month (no false empty claims) and measured 480px saved full card (scrollable Week, view controls retained).
- Focused validation: CalendarCard + useCalendarEvents suites **26/26 passed**, 2 files, 1.50s. Existing act deprecation warning only. `git diff --check` passed. No cargo, UI control, runtime/config changes or World edits in this pass. Source is frozen for parent's native build.
