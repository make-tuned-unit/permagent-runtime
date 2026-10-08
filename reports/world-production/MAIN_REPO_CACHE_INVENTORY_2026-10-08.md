# Main-repository cache inventory — October 8, 2026

Read-only inventory requested to find approximately 3 GiB for the existing 25 GiB runtime guard. **That capacity was not found in the authorized scope. Nothing was deleted.** Scope was `/Users/j/Documents/dev/permagent-runtime` and task-named entries under `/private/tmp`; no global home/cache traversal, other-lane worktree/build-target inventory, benchmark inventory, or source/data cleanup was performed.

Main repository allocated size from `du -x -k -d 1`: **4,598,844 KiB (4.386 GiB)**. Most is protected source/data, Git history, packaging resources or installed dependencies. The main `target/` is only 74,300 KiB; there is no large `debug/` or `release/` tree left there.

## Exact cache/artifact candidates and exclusions

Times are latest contained-file modification timestamps in UTC, **not verified last-use timestamps**. Access time is not reliable evidence of inactivity. Logical/allocated sizes below are from file stat records; filesystem snapshots or shared extents can reduce actual reclaimed capacity.

| Exact path | Type / size | Latest file mtime | Activity and recommendation |
| --- | --- | --- | --- |
| `/Users/j/Documents/dev/permagent-runtime/target/sherpa-onnx-prebuilt/sherpa-onnx-v1.13.4-osx-arm64-shared-lib.tar.bz2` | Ignored downloaded dependency archive; 15,741,101 logical bytes / 15,745,024 allocated bytes (15.02 MiB) | 2026-09-11 06:40:31 | No target open-file rows in current `lsof` snapshot. Duplicate extracted distribution exists beside it. Small recoverable download-cache candidate; deletion would require reacquisition if archive needed later. Not deleted; cannot materially solve 3 GiB requirement. |
| `/Users/j/Documents/dev/permagent-runtime/target/sherpa-onnx-prebuilt/sherpa-onnx-v1.13.4-osx-arm64-shared-lib` | Four extracted build-dependency files; 60,322,376 logical / 60,334,080 allocated bytes (57.54 MiB) | 2026-09-17 05:18:15 | No target open-file rows at inspection, but this is a dependency distribution, not stale compiled intermediates. Preserve unless coordinator intentionally plans reacquisition and establishes no pending build needs it. |
| `/Users/j/Documents/dev/permagent-runtime/ui/command-center/node_modules/.vite` | Vite/Vitest cache, 835 files; 62,187,505 logical / 64,114,688 allocated bytes (61.14 MiB) | 2026-10-08 14:55:57 | **Recent/current, preserve.** Two live Node/esbuild process families use this same installed dependency tree. No direct `.vite` open file is insufficient to establish inactivity: dev servers open transformed files on demand. |
| `/Users/j/Documents/dev/permagent-runtime/ui/node_modules/.vite-temp` | Empty generated directory, 0 bytes | directory mtime 2026-09-18 09:58:16 | No open-file rows; no useful recovery capacity. |
| `/Users/j/Documents/dev/permagent-runtime/.pnpm-store` | One 4,096-byte file; effectively empty store | 2026-08-30 04:17:28 | No useful recovery capacity. |
| `/private/tmp/permagent-world-20261007` | 25 task logs, patches and worker handoff records; 726,526 logical / 790,528 allocated bytes | 2026-10-08 01:40:10 | No open-file rows at inspection. This is QA/handoff evidence, not disposable cache; preserve. |

Other matching `/private/tmp` entries were small task cleanup recipes/receipts and a backend patch (`permagent-repo-cleanup-20261007.{json,py}`, `permagent-final-backend-fixes.patch`). They are evidence/source patches, not cache candidates. No broad `/tmp` content traversal was performed.

## Protected larger directories

| Exact path | Allocated size (`du`, KiB) | Why excluded |
| --- | ---: | --- |
| `/Users/j/Documents/dev/permagent-runtime/.git` | 1,181,848 | Repository history and worktree metadata; no pruning proposed. |
| `/Users/j/Documents/dev/permagent-runtime/ui/command-center/node_modules` | 606,348 | Installed shared dependencies actively mapped by Node/esbuild; also contains the recent Vite cache above. |
| `/Users/j/Documents/dev/permagent-runtime/ui/node_modules` | 132,284 | Installed workspace dependencies, not a download store; absence of an open file would not establish no linked consumers. |
| `/Users/j/Documents/dev/permagent-runtime/documentation` | 515,604 | Documentation source/media (including large blog videos); there is no `documentation/node_modules` cache here. |
| `/Users/j/Documents/dev/permagent-runtime/.playwright-mcp` | 484,788 | QA evidence, excluded as data. |
| `/Users/j/Documents/dev/permagent-runtime/unreal` | 429,480 | Almost entirely `SolarForum/Content` source assets (429,276 KiB), not DerivedDataCache. Saved is only 176 KiB and includes config/autosaves; preserve. |
| `/Users/j/Documents/dev/permagent-runtime/assets` | 336,844 | Source/art assets. |
| `/Users/j/Documents/dev/permagent-runtime/recovery` | 314,796 | Recovery data, never treated as cache. |
| `/Users/j/Documents/dev/permagent-runtime/ui/desktop/src-tauri/resources` | 221,860 | Bundled MCP runtime/Whisper resources needed for shipping. |
| `/Users/j/Documents/dev/permagent-runtime/.hermit` | 110,844 | Toolchain executables; not identified as a stale task cache. |

## Open-file evidence and limitations

`lsof -n -P -Fpn` completed with exit 0. Output was filtered before display to the exact main-repository dependency/target/toolchain prefixes; unrelated process paths were not retained. Observed active mappings:

- PID 44182: command-center `fsevents/fsevents.node` and `@rollup/rollup-darwin-arm64/rollup.darwin-arm64.node`.
- PID 44186: command-center `@esbuild/darwin-arm64/bin/esbuild`.
- PID 60737: the same fsevents and Rollup native modules.
- PID 60738: the same esbuild executable.

Separate recursive `lsof +D` checks yielded no rows for the main target, task temporary directory, `.vite` or `.vite-temp`. These are point-in-time observations, not proof of no future use. `git check-ignore -v` confirmed target/node_modules/Saved are ignored; ignore status alone is not deletion authorization or proof of safety.

No deletion, process stop, model operation, build, package install or guard change occurred. The only clear small duplicate-cache candidate is the approximately 15 MiB Sherpa download archive. Even aggressively reclaiming the optional 57.54 MiB extracted dependency would remain far below 3 GiB and adds avoidable rebuild risk. Retain the resource guard rather than manufacture capacity from source, evidence, active dependencies or another lane.
