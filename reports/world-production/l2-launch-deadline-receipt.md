# L2 startup — whole-launch recovery deadline

2026-10-13. Working-tree change against `b263082c11c1f1fc58971872cbb951f64fa6debc`; no commit or index writes. Addresses Astra round-3 item 2, “Whole-launch recovery deadline”. Read `af9e827d`, `49ca85d2`, and `90a8e22e` before editing.

`main()` now records `SystemTime::now()` as Unix epoch milliseconds in its first statement. A small inline `launch-clock` Tauri plugin injects the immutable `window.__PERMAGENT_LAUNCH_EPOCH_MS__` property through `tauri::plugin::Builder::js_init_script`. This initialization hook covers the existing config-created main window and executes before HTML/page scripts; it avoids post-load `eval` and an asynchronous IPC round trip. Confirmed the API and webview integration in the pinned [Tauri 2.11.0 plugin source](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.0/crates/tauri/src/plugin.rs) and [webview manager](https://github.com/tauri-apps/tauri/blob/tauri-v2.11.0/crates/tauri/src/manager/webview.rs). The window still shows in setup; daemon startup still runs from `RunEvent::Ready`.

BootScreen initializes `deadline = launchEpochMs + BOOT_DEADLINE_MS`, keeping `BOOT_DEADLINE_MS = 10_000`. Without an injected native epoch, it uses `performance.timeOrigin`. Countdown, request checks, and the independent abort/recovery timer use that deadline. The countdown displays tenths of a second so the spent shell budget is visible. An already-expired deadline renders recovery immediately and starts no request. Recovery bypasses the entry fade. Explicit Retry starts a fresh ten-second window at the click. The two-second per-attempt abort, late-answer suppression, callback/unmount cancellation, and `daemon-failure` detail remain intact.

Five added regression tests cover:

- Launch 1.5 seconds before mount: initial `8.5s remaining`, per-attempt abort at two seconds, no Retry one millisecond before launch + ten seconds, enabled Retry exactly at that deadline, and no routing from a late answer.
- Launch eleven seconds before mount: visible recovery and enabled Retry immediately, without a connection request or fade.
- Web build without Tauri or the native global: countdown and deadline derive from `performance.timeOrigin`.
- Explicit Retry: a fresh ten-second budget, no background retries after failure, and successful recovery on a later Retry.
- Remount: the original process deadline persists, with the active short-budget request aborted at that deadline.

Gates run from `ui/command-center` unless noted:

- Focused serial run of `src/components/splash/BootScreen.test.tsx` and `src/App.startup.test.tsx`: **24/24 passed**, two files, exit 0 (17 BootScreen tests, seven App startup tests).
- `npx tsc --noEmit`: **final run exit 0**. An earlier run caught `SettingsView.tsx(2156,103): TS2339: Property 'accent' does not exist on type 'ThemeColors'`; the settings owner corrected it during this task. That file was not edited by this task. An initial new-test use of `Array.at` was corrected for the existing TypeScript target before the full suite run.
- Full serial command: `node --no-concurrent-sparkplug node_modules/vitest/vitest.mjs run --config ../../reports/world-production/vitest.serial-forks.config.ts`: **exit 1; 4,482 passed, three failed (4,485 total); 542 files passed, two failed (544 total)**. Duration 400.87 seconds. All startup tests passed. The three failures were outside the allowed startup scope, in concurrent `SettingsView.tsx` changes: `spaceScale.test.ts` reported `marginLeft: 8` at line 2156; `rawButton.test.ts` reported raw buttons at lines 693/2152 and an exception count of two instead of one.
- After the settings owner switched the new control to `Button`, a focused serial rerun of `src/styles/spaceScale.test.ts src/components/common/rawButton.test.ts` returned **six passed, one failed (seven total), exit 1**. Both Button-rule failures cleared. The remaining failure is `SettingsView.tsx:2152 marginLeft: 8`; its owner must use the existing spacing token. This task did not alter that out-of-scope file. The full-suite gate is therefore **not green**.
- From the worktree root, `rustfmt --check --edition 2021 --config skip_children=true ui/desktop/src-tauri/src/main.rs ui/desktop/src-tauri/src/daemon.rs`: **exit 0**. The explicit edition matches Cargo.toml; the initial invocation without it defaulted to Rust 2015 and could not parse existing async/C-string syntax. `skip_children` limits checking to the requested two files.
- Scoped `git diff --check`: **exit 0**.

The serial runner emits a sandbox `EPERM` warning for Vite's WebSocket listener on port 24678, plus React/jsdom warnings. Runs complete despite those warnings; the full run's nonzero exit is from the three assertions detailed above.

Changed source files (and the complete contents of the patch):

- `ui/desktop/src-tauri/src/main.rs`
- `ui/command-center/src/components/splash/BootScreen.tsx`
- `ui/command-center/src/components/splash/BootScreen.test.tsx`

`daemon.rs` and `useDaemonFailure.ts` were read and left unchanged. No helper file was needed. Other owners' settings changes and pre-existing packaging reports are excluded from the patch.

Artifacts:

- `reports/world-production/l2-launch-deadline.patch`
- `reports/world-production/l2-launch-deadline-receipt.md`

Patch SHA-256: `107c9b59738c583a329b4a3d5bd07156dd6124c443d34ae21a5a88ba96af2ce7`.

No Cargo/Tauri build, native launch, or install was performed. Coordinator command, from the worktree root:

```sh
cargo check --manifest-path ui/desktop/src-tauri/Cargo.toml --no-default-features
```

The coordinator still needs to compile and measure launch, first feedback, actionable Retry, and successful recovery on the final native candidate. Unit-test timestamps establish the deadline logic; they are not a substitute for Astra's requested native timing receipt.
