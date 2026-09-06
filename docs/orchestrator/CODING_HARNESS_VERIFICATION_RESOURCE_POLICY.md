# Coding Harness Verification Resource Policy

**Date:** 2026-09-04 (America/Halifax)  
**Purpose:** preserve rigorous evidence without turning every focused change
into an unbounded compile, disk, or retry loop.

## Incident evidence

A filtered `permagent-daemon` test command without a target selector linked
nineteen unrelated integration-test executables of 547–656 MiB each. Available
disk fell below 1 GiB. Removing only those named disposable binaries recovered
approximately 11 GiB. A later correctly scoped `--lib` filter compiled one
655 MiB monolithic daemon unit-test executable, but macOS killed it with
SIGKILL before any assertion ran.

These are verification-infrastructure failures. They must not be labelled as
product assertion failures, retried unchanged, or used as evidence that a
feature passed.

## Required verification ladder

Each child DAG declares the smallest evidence ladder that can falsify its
change. Run it in order and stop at the first failing gate:

1. format/parser and `git diff --check` for touched files;
2. pure unit tests in the smallest owning crate/target;
3. typecheck or `cargo check -p <crate> --lib`;
4. one named integration target (`--test <name>`) when the behavior crosses a
   real boundary;
5. product build and simulator/device/system E2E only at the child’s fan-in or
   release gate.

Never use `cargo test -p <crate> <filter>` for a crate with many integration
targets: Cargo may build every target before applying the runtime filter. Use
`--lib`, `--bin`, or `--test <name>` explicitly. The 2026-09-05 diagnostic
resolved the daemon SIGKILL as an invalid generated ONNX dylib signature,
not evidence that the `--lib` binary is too large. Seven projection tests
executed after narrow signature repair (see `END_TO_END_RUNTIME_GATE_RECOVERY.md`).
Do not extract production logic solely to work around that disproven diagnosis.
For focused macOS daemon unit checks use `scripts/test-daemon.sh --lib <filter>`:
it builds only that target, signs/verifies the generated dylibs, then executes
the same scope. Omitting `--lib` intentionally retains full lib/integration
coverage and is appropriate only when that broader gate is required.

## Retry and resource bounds

- Maximum two test-fix cycles per unchanged child gate.
- A command killed before an assertion runs is `infrastructure_failed`, not
  `test_failed`; collect executable size, free disk, and command shape once.
- Do not repeat the same command until code, target selection, resource state,
  or test infrastructure has materially changed.
- Only one link-heavy Rust verification command may run against the shared
  target directory at a time.
- Capture free disk before link-heavy gates. Below 8 GiB, clean only resolved,
  disposable artifacts from prior commands or block the gate with evidence.
  Keep units explicit: `df -k` availability is KiB, and 8 GiB is 8,388,608
  KiB. A 16,777,216 threshold belongs to 512-byte blocks, not `df -k`.
  Refuse malformed readings before starting Cargo; do not classify a unit
  conversion error as an actual shortage or use it to justify deletion.
- Never delete source, the workspace, or a broad target tree automatically.
  Resolve exact generated paths first and report what was removed.

## Evidence vocabulary

Every verification receipt records one of:

- `passed`: assertions executed and passed;
- `test_failed`: an assertion or compile/type error attributable to the code;
- `infrastructure_failed`: assertions did not run because of tooling/resource
  failure;
- `not_run`: outside this child’s gate;
- `deferred_to_fan_in`: deliberately owned by the named integration/E2E node.

A child may pass implementation with an infrastructure defect only when its
contract has other executable checks and the missing boundary test is carried
as a named dependency of a later fan-in gate. Release qualification cannot
inherit that exception.

## Harness instrumentation

Record per command: DAG/node ID, target selector, start/end, exit/signal,
assertions discovered/executed, wall time, peak RSS when available, disk delta,
and whether source changed since the preceding attempt. The orchestrator uses
this receipt to reject unchanged verification loops and to choose the next
smallest meaningful gate.
