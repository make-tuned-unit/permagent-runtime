# Combined QA, UX, and Grow integration — 2026-09-26

Integration checkout: `/Users/j/Documents/dev/permagent-worktrees/dagJ-lane-qa-int3`, branch `dagj/qa-int3`. This is the single source for the combined release. Original recovery source worktrees were preserved. No installed app or running daemon was replaced or restarted.

## Integrated changes

- `ef11f4f4` merges full UX tip `400ef0a2` into QA `6b12bb4f`. Anthropic keeps the QA rolling-prefix production fixes and regressions plus the UX large-prefix/short-turn regression. Notifications retain returned IDs alongside absolute expiry/dismiss callbacks. QA human-unblock progress resets and UX worker-plan evidence both survive.
- `535aa727` makes release sidecar staging honor `CARGO_TARGET_DIR`, preventing accidental staging from another target. Shell syntax and selected-path checks pass.
- `70696088` removes byte slicing from Ollama parameter-size parsing after the combined strict gate found three Clippy errors.
- `23bd80ff`, `86507b8b`, `21365fdf` integrate rate-limit reservation recovery and real ProviderRetry/SQLite regressions. Confirmed pre-output transport rejections release holds; ambiguous failures, accepted requests, and partial output retain conservative accounting. The final commit fixes test-only trait visibility.
- `6734993d` integrates the planner-cost fix. The Grow branch's older multi-path rejection was intentionally omitted because QA already supports multiple existing paths, preserves real filenames with spaces, and accepts arrays with a provider-compatible string schema.
- `be69cf23`, `e62f2221` add actionable isolated-worker dependency setup and consistent declared/available package-manager and Corepack selection.
- `88c3c550`, `2f92aac1`, `722f7c1f` preserve the bounded original launch contract, reload durable task-budget identity, and bind the actual new user contract atomically to task ingress. Reused sessions cannot silently substitute an old task's first message. Existing budget IDs retain their schema; oversized ordinary chat remains accepted.
- `19c6d4b2` prevents unverified harness exits from marking a Grow action implemented. Explicit human completion remains intentional.
- `b09f940f`, `715036ea` correct the previously unexecuted QA approval regression fixture: parking is the Failed lifecycle state, and choice decisions require detail. The actual approval/Ready/reset assertions remain intact.
- `42248f62` preserves completion-check provenance through redispatch. Persisted generated checks and unknown legacy checks cannot become user-authored merely because they already exist.

## Verification

- Combined UI production build passed, including TypeScript. Full UI suite: **442 files / 3,491 tests passed**. Subsequent UI change was an API documentation comment only.
- Strict core/daemon gate passed on final code checkpoint `42248f6261befb7d4963fafdafc5e58ff0abbebb`:
  `cargo clippy -p permagent -p permagent-daemon --lib --bins --tests --jobs 1 -- -D warnings`.
- Separate strict `permagent-eval` bin/test gate passed, covering the QA benchmark regrade target omitted by the core/daemon command.
- **219 targeted Rust tests passed**: 216 core tests and 3 eval regrade tests. This is targeted execution, not a claim that all 6,312 core tests ran.
  - 160 unchanged core tests passed at `722f7c1f`: reservation/transport accounting, planner complexity, multi-path search, Anthropic cache formatting, Ollama window behavior, zero-progress handling, worker plans, dependency setup, contract identity, and task-bound contract persistence.
  - 56 final tests passed at `42248f62`: corrected command-approval regression (1), SQLite redispatch provenance regressions (2), and the complete verification-approval module (53).
  - Regrade tests cover missing judge verdicts, existing run-directory rules, and companion result sidecars.
- Formatting and diff checks pass. The two approval fixture failures were diagnosed and corrected without weakening production guards or regression assertions.

All test execution used temporary roots. No paid model request or live Grow action was performed by this integration worker.

## Normal CLI runtime handoff

A normal CLI binary was built from clean source checkpoint **`42248f6261befb7d4963fafdafc5e58ff0abbebb`**:

- Path: `/private/tmp/qa-ux-target/debug/permagent`
- SHA-256: `469f68a1214578c1e916144238c5313647d2b508847bbb7ebc471ee3b821d278`
- Size: 582,937,032 bytes
- Build: `cargo build --profile test -p permagent-cli --bin permagent --jobs 1`
- Isolated-root smoke: `--version` reports `1.31.0`; `run --help` exits 0. No DYLD override is required.

The test profile reuses compatible dependency artifacts; this is the normal CLI main executable, not a test harness. It is a development validation binary, not an optimized signed app release. The linker reported a nonfatal large `__eh_frame` compact-unwind warning; compilation and runtime smoke succeeded. The strict source gates did not report Rust warnings.

The coordinator and Grow worker received the exact path/hash/source. The coordinator subsequently authorized the separate CLI-only authentic Grow run. That run's results belong to the Grow report; they are not claimed here. Heavy-build ownership was handed to the Grow npm/test phase after this checkpoint.

## Resources and artifact provenance

The UX thread explicitly handed over exclusive `/private/tmp/qa-ux-target` ownership. The guard's shared-target override implements that transfer; gate logs identify this integration checkout's package paths, and linked test binaries have recorded source/hash manifests. No concurrent compiler used the target.

Initial free disk was approximately 5 GiB. Rust checks used a 3 GiB entry floor; measured serial executable builds used a **4 GiB entry guard and a monitored 2 GiB stop floor**, `CARGO_INCREMENTAL=0`, and one job. Builds were stopped before unsafe link phases and resumed from completed dependencies. No fresh release profile was attempted. CLI handoff left approximately **3.8 GiB free**.

The coordinator authorized exact-manifest reclamation of obsolete generated artifacts: old orphan/split-debug objects, obsolete standalone test executables, and stale main-target static archives with validated newer QA counterparts. Source, current dependency artifacts, runtime binaries, and user data were preserved. Current tested standalone core binaries and their exact test-only codegen objects were hash-recorded before reclamation for subsequent links. This trades regenerable old debugging/cache material for space; it is not a blanket target clean.

Evidence directory: `/private/tmp/qa-ux-recovery`.

- `combined-ui-build.log`, `combined-ui-test.log`
- `combined-rust-clippy-provenance.log`, `combined-rust-clippy-eval.log`
- `core-regression-manifest.json`, `final-core-regression-manifest.json`, `combined-rust-eval-tests.log`
- `combined-cli-build-final.log`, `combined-cli-provenance.json`
- `RCGU_ORPHAN_CLEANUP.json`, `RCGU_STALE_CLEANUP.json`, `OLD_TEST_EXECUTABLE_CLEANUP.json`, `MAIN_ONLY_STALE_ARTIFACT_CLEANUP.json`, `MAIN_STATIC_ARCHIVE_CLEANUP.json`, and exact current-test cleanup manifests

## Remaining gates

- The Grow HTTP integration binaries `growth_actions_wiring` and `growth_verify_completes` were compiled/linted by the strict daemon test gate but have not been linked and executed. They must run in a later idle/resource-safe slot.
- Native owner, Brain forget/Undo, Librarian answer, relationship, and other daemon/app integration still need isolated runtime acceptance. Prior UX browser checks used mocked APIs.
- No combined daemon binary, Tauri bundle, signed app, or updater release was built/installed in this pass. The production UI build uses the web `/ui/` base; Tauri packaging must rebuild with `TAURI_ENV_PLATFORM` for the native `/` base.
- Packaged release acceptance, the previously planned benchmark baseline, and live Grow evidence remain separate pending work. No release is claimed complete.
