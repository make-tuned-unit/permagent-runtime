# Native staging plan — 6 October 2026

## Verified frontend baseline

Lane `world/production-20261006`, starting HEAD `87aa520917cc92a15a30bd6218d62787500d3024`. Full current `tsc --noEmit` passed after three scoped World typing fixes. Sequential Vite production build passed,3104modules/8.53s; dist626files/80,822,031bytes. Current v3b runtime kit hash e114f28bedaafa2f337c9d0b0e893e3b8f03a337d37bd9f9d177ccaff30d2300; preserved v2 hash99a1ad1632e4a8adc8820767f6b684321a37b945692d3aa019e1c9d3a7f57139. Focused7/7tests passed2.61s after typing fixes. Build warnings: stale Browserslist data and existing chunks over500kB. No package update attempted.

Commands ran from `ui/command-center` through absolute lane `scripts/build-guard.sh --`, with `CARGO_TARGET_DIR=/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006` to honor lane isolation:

- `node node_modules/typescript/bin/tsc --noEmit` (Node2048MB,300s watchdog)
- `node node_modules/vite/bin/vite.js build` (Node2048MB,600s watchdog; monitored10GiB diskfloor)
- `node node_modules/vitest/vitest.mjs run src/components/world/forum/staticGardenInstances.test.ts src/components/world/forum/compactGarden.test.ts src/components/world/forum/EvidenceLesson.test.tsx --maxWorkers=1 --minWorkers=1` (Node768MB,120s watchdog)

The separate typecheck and Vite command execute both components of the repository `build` script without duplicating its already-passed tsc stage.

## Cache and dependencies

Free disk about36GiB at checks; installed app baseline `/Applications/Permagent.app`964MiB, untouched. Contents: MacOS715MiB, Resources217MiB, Frameworks31MiB. Installed app executable99,762,016bytes, daemon369,375,744bytes and CLI280,369,088bytes. Those binaries are baseline resources, not new World compilation evidence.

Main checkout standalone native target has888MiB release output, including a59,048,976byte old `permagent-app`. No compiled `libtauri`/`libtauri_build` dependency libraries were found there. Harness shared debug target has4.6GiB deps but no Tauri compiled libraries/fingerprints. Its whole14GiB target is not an appropriate seed. The old app/fingerprints cannot verify new embedded frontend, toolchain or lane; do not run them as evidence or write into another lane's target. Rust1.97.1, aarch64-apple-darwin is installed.

`cargo metadata --manifest-path ui/desktop/src-tauri/Cargo.toml --locked --offline --format-version 1` stopped because `adler2 v2.0.1` is absent. Dependency fetching will be necessary for a native build; no native compiler or dependency fetch has started. Rust registry caches may provide some archives, but reuse is unproven. A cold native dependency compilation budget is required.

## Proposed isolated heavy phase

1. Recheck no other cargo/rustc/asset jobs, free disk>=10GiB and memory pressure. Keep one compile worker. Preserve parent's storage cleanup review and unrelated changes.
2. Use the lane-only target above. Do not copy the14GiB shared target. Materialize ignored native `binaries/` and `resources/` prerequisites from the installed app baseline, preserving source paths/hash ledger: baseline daemon/CLI under `*-aarch64-apple-darwin`, existing Frameworks dylibs and Resources MCP/whisper content. The lane currently lacks those generated inputs. Native build is only the frontend/native shell; daemon/CLI/model code is unchanged.
3. Use a temporary JSON config file with `build.beforeBuildCommand` empty (already verified dist) and `bundle.createUpdaterArtifacts=false`; retain actual app security/window settings. Build the ARM64 native shell with the installed Tauri CLI, debug profile, no installer/bundler, locked Cargo dependencies, one worker. Suggested environment `CARGO_BUILD_JOBS=1`, `CARGO_PROFILE_DEV_DEBUG=0` reduces symbols without changing app features. CLI confirmed `--debug`, `--no-bundle`, `--config` and forwarded Cargo args are available. Proposed command from `ui/desktop`: `node ../command-center/node_modules/@tauri-apps/cli/tauri.js build --debug --no-bundle --config /tmp/world-native-build-config.json -- --locked -j 1`, wrapped in repository guard using the lane target. First allow necessary public Cargo dependency fetching. Enforce30minute process-group watchdog and a continuously monitored10GiB diskfloor; stop owned build only if either limit fires.
4. Create a staged app under this lane's ignored native output, for example `.../.shared-target/world-production-20261006/staged/Permagent World Review.app`, copy the existing installed bundle, replace ONLY its staged `Contents/MacOS/permagent-app` with the newly compiled binary. Keep baseline sidecars/resources/frameworks. Ad-hoc-sign only the staged bundle with existing local entitlements as needed; do not modify `/Applications`, install, notarize, upload, update keys or create updater artifacts. Record binary/dist hashes, architecture, dependency resolution and staged bundle verification.
5. Native launch must account for single-instance behavior: current code registers the single-instance plugin and original identifier `ai.permagent.desktop`, so opening a second bundle can redirect to the installed app. Coordinate the parent's controlled native review/active app state before launching. An unchanged installed window is not evidence of new World assets. UI review must use authorized native CUA; do not bypass browser restrictions. No synthetic FPS/final-landscape claim.

## Estimates and limits

This is a conservative planning estimate, not measured build demand:1–4GiB native compile output with debug symbols disabled,0.5–1GiB registry/download/unpack growth,~0.9GiB ignored baseline inputs,~1–1.3GiB staged bundle. Reserve8GiB incremental headroom for link/rebuild uncertainty in addition to the10GiB floor; current~36GiB is sufficient if no competing heavy task consumes it. Repeated release/debug variants or full daemon/CLI compilation are outside this phase. Cold native build might take10–30minutes and up to2–4GiB RAM for one rustc/link worker; stop/replan if actual behavior exceeds guard. No native heavy phase has launched at this audit checkpoint.

Frontend source/build availability is now verified. Actual integrated visual/motion/alpha stability, full-land free roam, frame-time/memory and final visual acceptance remain open.
