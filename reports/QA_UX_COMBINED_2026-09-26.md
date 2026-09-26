# Combined QA, UX, and Grow integration — 2026-09-26

Integration checkout: `/Users/j/Documents/dev/permagent-worktrees/dagJ-lane-qa-int3`, branch `dagj/qa-int3`. Original recovery worktrees remain preserved. This checkout is the single release source; no app or daemon has been replaced or restarted.

## Integration

- `ef11f4f4`: merged full UX tip `400ef0a2` into QA `6b12bb4f`. Anthropic production code and QA rolling-prefix regressions remain intact, alongside UX large-prefix/short-turn regression. Toast returns its notification ID and also carries the absolute expiration deadline and dismissal callback. QA human-approval zero-progress reset and UX worker-plan evidence both survive.
- `535aa727`: sidecar staging honors `CARGO_TARGET_DIR`; otherwise an isolated release could silently stage binaries from the wrong target. Shell syntax check passes; missing-binary probe confirms the selected target.
- `70696088`: replaced byte indexing with a character iterator in QA Ollama parameter-size parsing, resolving three strict Clippy string-slice errors.
- `23bd80ff`: Grow reservation accounting checkpoint (`5e64d1a8`).
- `6734993d`: Grow planner-cost checkpoint (`aba3dab0`). Its search-path rejection was intentionally omitted: the QA implementation already supports multiple existing paths while preserving filenames with spaces, and accepts JSON arrays. The older Grow change would remove that recovered behavior.

## Verification

At combined QA+UX merge (before Grow followups):

- UI production build passed.
- Full UI suite passed: **442 files, 3,491 tests**.
- `cargo fmt --all -- --check` passed.
- First combined strict Clippy run rebuilt `permagent` from this checkout and failed on exactly three byte-slice warnings in the recovered QA Ollama parser. Those warnings were fixed in `70696088`; final Rust gate awaits batching remaining Grow checkpoints.

Logs: `/private/tmp/qa-ux-recovery/combined-ui-build.log`, `combined-ui-test.log`, `combined-fmt.log`, `combined-rust-clippy.log`.

## Build ownership and pending runtime validation

Exclusive ownership of `/private/tmp/qa-ux-target` was handed off by the UX thread in `/private/tmp/qa-ux-recovery/COORDINATION_TO_QA_WALK.md`. It is a warm debug target, not a release target. The guard's shared-target override is used only for this explicit ownership transfer. The Clippy log identifies the integration checkout's own package paths; no other agent may build into or modify this target.

Only approximately 5 GiB disk space remains. Check/lint uses the existing documented 3 GiB floor with `CARGO_INCREMENTAL=0`; a fresh release profile, Rust test executable linking, or daemon linking must pass the normal build guard/resource assessment before starting. No clean/delete operation or lower release threshold was used. Rust test execution, live isolated-daemon Grow verification, native packaged-app validation, and installation remain pending.
