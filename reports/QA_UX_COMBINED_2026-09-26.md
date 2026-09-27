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

## Workspace scope follow-up

The first authentic Grow trial exposed conflicting original-checkout paths before the worker's execution scope. `594de8e3` (Grow `bd67366a`) leads both internal and external worker prompts with the actual writable worktree, labels historical task paths as source-reference context, and maps repository-relative setup, edits, and tests to that worktree. The original task contract remains byte-for-byte unchanged and the write jail remains enforced.

Strict core/daemon Clippy passed again with `-D warnings`. Four additional actual regressions passed at `594de8e3`: execution scope precedes and preserves original paths, the worktree is the only writable root, the compact prompt stays within 6,000 tokens, and local-window sizing counts the real worker brief. This brings targeted executed tests to **223** (220 core and 3 eval). Test linking took 2m28s; normal CLI rebuilding took 2m01s. The only linker warning remains the previously acknowledged compact-unwind size warning.

The replacement normal CLI at `/private/tmp/qa-ux-target/debug/permagent` is from source **`594de8e3d837b8daa30a0afd162fb5599f48b8a4`**, SHA-256 **`ea199658a6fdac985177130958a5dc5dc53c6b4b8544d5dbbe078257dc620348`**, 582,938,440 bytes. Isolated-root `--version` and `run --help` pass without a DYLD override. Handoff left 3.71 GiB free. The subsequent authorized trials are recorded below; none achieved a landed implementation.

For this rebuild, the coordinator authorized exact removal of the invalidated core runtime archive and its 16 objects, then the successfully tested standalone test executable and its 16 test-only objects. Full hashes, stat identity, idle/open-handle checks, and test evidence were retained; dependencies and the existing CLI were preserved until replacement linking completed. Evidence: `workspace-scope-clippy.log`, `workspace-scope-test-build.log`, `workspace-scope-regression-manifest.json`, `workspace-scope-cli-build.log`, `workspace-scope-cli-provenance.json`, `current-core-runtime-rebuild-cleanup.json`, and `workspace-scope-tested-artifacts-cleanup.json`.

## Initial normal CLI runtime handoff (superseded by follow-up)

A normal CLI binary was built from clean source checkpoint **`42248f6261befb7d4963fafdafc5e58ff0abbebb`**:

- Path: `/private/tmp/qa-ux-target/debug/permagent`
- SHA-256: `469f68a1214578c1e916144238c5313647d2b508847bbb7ebc471ee3b821d278`
- Size: 582,937,032 bytes
- Build: `cargo build --profile test -p permagent-cli --bin permagent --jobs 1`
- Isolated-root smoke: `--version` reports `1.31.0`; `run --help` exits 0. No DYLD override is required.

The test profile reuses compatible dependency artifacts; this is the normal CLI main executable, not a test harness. It is a development validation binary, not an optimized signed app release. The linker reported a nonfatal large `__eh_frame` compact-unwind warning; compilation and runtime smoke succeeded. The strict source gates did not report Rust warnings.

The coordinator and Grow worker received the exact path/hash/source. The coordinator subsequently authorized the separate CLI-only authentic Grow run. That run's results belong to the Grow report; they are not claimed here. Heavy-build ownership was handed to the Grow npm/test phase after this checkpoint.

## Authentic Grow outcomes and current stopping point

Three coordinator-authorized CLI-only trials used the real CityCircle task in fresh isolated runtime roots and product-created goal worktrees. No daemon or installed runtime changed. The second and third trials used the current tested CLI from `594de8e3`; the strict, UI, and four scope-regression evidence above remains applicable.

| Trial | Outcome | Recorded actual USD | Unresolved hold USD |
| --- | --- | ---: | ---: |
| 1 | Original checkout paths confused the worker; write jail refused the wrong-root operations. Stopped before implementation. Scope priority subsequently fixed and tested in `594de8e3`. | 0.117368955 | 0.1331532 |
| 2 | Scope fix worked: observed edits stayed in the worktree. Candidate `91902ff` was rejected before landing despite 9 existing test files / 138 tests passing. | 0.192016140 | 0.1406997 |
| 3 | OpenAI accepted-stream TPM error produced no matching authoritative usage receipt; conservative unknown hold exhausted available reservation headroom. Goal failed/parked and CLI exited without implementation. | 0.086400300 | 0.4126065 |

Trial 2 added no behavioral tests. Review reproduced incorrect quartiles (25 at every reached threshold), found scroll tracking skipped when CTA cleanup returned early, incorrect below-fold classification after scrolling, and missing session/event persistence and crossed-quartile behavior. Its nonexistent `npm run check` was masked by a pipeline; lint failed and no production build ran. Candidate commit/worktree is preserved as rejected evidence, not shipped work.

Trial 3 used OpenAI gpt-5.4-mini for parent and native worker with auto-land disabled and a $0.75 cap. It installed dependencies and read project docs but made no code changes. The in-band rate-limit event reported a 200,000 TPM limit, 166,057 used, and 36,302 requested, with retry-after 707 ms and token reset 49.816 seconds. Accepted transport plus absent usage evidence kept the two-attempt $0.4126065 envelope UNKNOWN. The next roughly $0.41 reservation, and then parent continuation, correctly failed the budget gate. No unknown hold was released or presented as measured spend.

Aggregate receipt: **$0.395785395 actual**, **$0.6864594 unresolved holds**, leaving **$0.417755205** of the original $1.50 authorization envelope. Unresolved liabilities remain preserved. Actual CityCircle stayed clean at `a9ba56ee2f6e3ec642b86cb4cf3ec5fa52b1ef46`; no candidate landed, no live Grow verification ran, and nothing deployed.

Evidence: `/private/tmp/permagent-grow-test/RUN_RESULT.md`, `/private/tmp/permagent-grow-test-attempt2/RUN_RESULT.md`, `/private/tmp/permagent-grow-test-attempt3/RUN_RESULT.md`, and the third root's `aggregate-budget-receipt.json` and `reservation-failure-evidence.json`. Isolated credentials were removed and launched process groups exited.

No fourth paid run is authorized absent a real pacing mechanism or revised plan. Read-only source audit found `GOOSE_MAX_TOKENS=8192` is a supported fresh-session output cap, propagated to coordinator, planner, internal worker, reservation, and API request. It is not input-TPM pacing, and same-model resume can retain saved model configuration. No supported per-run token pacing setting was found; skip-backoff would remove waiting, not solve this failure.

The separate World worker exhausted its allowance and recorded checkpoint `eed56db6`, with no implementation. The heavy-build slot is idle. HTTP execution and native packaging remain pending; neither this integration nor the authentic trials establish end-to-end release success.

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
- Packaged release acceptance and the previously planned benchmark baseline remain pending. Live Grow trials are recorded above, but successful implementation/landing and live verification remain unmet. No release is claimed complete.
