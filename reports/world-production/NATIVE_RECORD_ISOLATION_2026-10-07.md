# Isolated native Record review target

Implementation is source-frozen. No app/daemon was launched, staged or stopped by this work. The only runtime-root mutation was creating the explicitly approved empty canonical `record-qa-runtime` directory during preflight. No production data/token/config was copied.

## Coherent target

`ui/desktop/src-tauri/src/runtime_target.rs` owns one immutable target. Without compiled overrides it retains `$HOME/.permagent` and loopback3001. Runtime environment variables cannot redirect a compiled app. A review build requires both `PERMAGENT_REVIEW_ROOT` and `PERMAGENT_REVIEW_PORT`; release builds reject them. Root must be absolute, canonical, separate from production state, and named `record-qa-runtime`. Port must be unprivileged, valid, and different from3001. Host is always127.0.0.1. The distinct compiled/app bundle identity is `ai.permagent.record-review-20261007`.

The following paths now share that target:

- `daemon.rs`: detection and token reads. Review mode waits only for the external selected-port daemon; it does not consult production launchd, spawn any sidecar, supervise/restart a daemon, or stop a daemon at app exit. The existing production path remains intact.
- `activity.rs`: daemon token plus activity POST.
- `browser.rs`: inbox directory, token, inbox metadata POST.
- `terminal.rs`: supervised-output POST, using the same target token. Arbitrary terminal commands are not an OS sandbox and are outside Record isolation acceptance.
- `main.rs`: validates review identity/root before backend initialization and writes heartbeat under the selected root.
- `build.rs`: validates the target and matching `VITE_DAEMON_URL`; Cargo tracks changes to all three compiled build inputs.

Three std-only target validation tests passed (direct rustc test binary, no Cargo dependency rebuild): unchanged production defaults; coherent isolated URLs/token root; invalid root/port, partial overrides, release overrides and production-root rejection. The full native build/IPC tests remain parent-owned and pending.

## Concrete build and evidence modes

`native-record-review-config.json` supplies the distinct bundle identity/title and CSP allowing localhost3002 HTTP/WS while removing localhost3001. The default `native-review-config.json` is unchanged. `native-record-review-target.json` records exact root/port/env expectations.

All three scripts now accept explicit `--record-review`; their default World mode remains available:

```
python3 reports/world-production/run_guarded_native.py --record-review record-r1
node reports/world-production/verify_native_review.mjs --record-review
python3 reports/world-production/stage_native_review.py --record-review \
  --audio-helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-scoped-review \
  --audio-helper-sha256 0318ce2682c8a1b917ba91e36c79911ee881c6d591a23298df014d0ed3e28fa7
```

First stage omits `--refresh`; subsequent refresh requires the Record review app closed. It stages `Permagent Record Review.app` separately from `Permagent World Review.app`. The shared Cargo raw output is still one `debug/permagent-app`; its exact hash is checked before staging, so stale World/Record evidence cannot authorize the other artifact.

The build runner removes inherited PERMAGENT_REVIEW_ROOT/PORT and VITE_DAEMON_URL/VITE_API_BASE_URL in both modes, then explicitly sets the three Record inputs only in Record mode. It still checks all baseline prerequisite hashes, requires25GiB startup/20GiB continuous free disk, and signals only its owned process group. It does not launch or stage.

Record verifier checks the current binary's Brotli-backed dist assets, exact identity/name, compiled isolated root, exact compiled CSP, presence of the3002 frontend URL, and absence of full localhost3001 HTTP/WS endpoint literals in the embedded frontend JS. A strict gate may identify an otherwise-unused Settings fallback literal; the frontend owner must remove/derive those, not weaken the verifier. Native production-default strings can still exist in shared native code; the verifier's no3001 rule applies to compiled review CSP and actual frontend assets.

Separate evidence files: `native-record-build-*.log/json`, `native-record-embedded-assets-verification.json`, and `native-record-package-verification.json`. Record staging requires the explicit signed scoped-helper override, validates its marker/hash, and preserves the existing installed baseline snapshots. It does not replace or launch a daemon. The copied bundled baseline daemon is inert because Record mode never spawns it.

Validation already performed: Python syntax, Node syntax, help output; full read-only/build-input preflight (plus allowed root mkdir) passed baseline hashes, root canonicality, identity/CSP and disk floor. No actual native build, embedded verification or stage has run in this phase.

## Daemon and provisioning prerequisites

Before launch, the parent must verify3002 is free and fail if occupied; never kill an existing listener or reuse a production bearer. Launch the newly built daemon by exact path in a separately owned process group, with a clean child environment and:

```
PERMAGENT_PATH_ROOT=/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/record-qa-runtime
PERMAGENT_DISABLE_KEYRING=1
<new-daemon> agent --host 127.0.0.1 --port 3002
```

No global HOME/audio routing changes are needed. Keep the newly generated isolated token at `<root>/secrets/daemon_token.json`; native IPC reads exactly that file. Do not import host provider/service credentials or production projects. `PERMAGENT_CONFIG`/`PERMAGENT_SPECTRAL_DB`, if supplied for observability, must also point inside this root; PATH_ROOT is the actual shared data-path control. Fresh config keeps opted-in background services unconfigured. This is application-state isolation, not a VM or general restriction on arbitrary terminal/calendar/system actions.

The daemon's default features include local-inference. A `--no-default-features` build must explicitly include it for local Whisper. Existing frontend resource resolution plus `/api/dictation/provision` copies the bundled `whisper/whisper-base-q8_0.gguf` to isolated `<root>/models/whisper-base-q8_0.gguf` and sets LOCAL_WHISPER_MODEL=base. The current provisioning API copies the supplied file without filename/hash validation; the isolated supervisor must validate the exact bundled filename and expected model SHA-256 before requesting that copy, then verify the resulting destination hash. No new native model-resolution command is necessary.

Live acceptance remains: selected app-only capture, native final-tail delivery, local Whisper, project-bound saved meeting, durable Orchestrator triage, deduplicated evidence-linked Kanban cards, and cleanup/retry checks. Confirm source/daemon/helper hashes and selected root/port in evidence; a passing source fixture or note-save toast alone is insufficient.

## Separate versioned stage while R1 remains running

Staging now optionally accepts `--record-review --stage-version record-r2`. This selects only:

- `../.shared-target/world-production-20261006/staged/Permagent Record Review (record-r2).app`
- `reports/world-production/native-record-package-verification-record-r2.json`

The default World and unversioned Record paths/reports remain unchanged. The stage label must be a lowercase `record-` label with letters/digits/single hyphens, at most40characters; traversal, whitespace, punctuation and symlink redirects are rejected. A version label does not change the compiled/app bundle ID or internal name: `ai.permagent.record-review-20261007` / `Permagent Record Review` remain the verifier contract. The parent should use the exact versioned path and quit the older same-identity app before opening this one.

After the current build and embedded verifier pass, the parent can stage R2 separately:

```sh
python3 reports/world-production/stage_native_review.py --record-review --stage-version record-r2 \
  --audio-helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-preflight-review \
  --audio-helper-sha256 490f4e56bda9a67ce96b2a62aa441f1ae2fc099a07c531ec38a3bd0c30cfc701
```

This first stage omits `--refresh`. It does not inspect/overwrite the running unversioned R1 stage or its ledger. If the selected version's bundle or ledger already exists, staging refuses by default. Explicit `--refresh` requires that exact version/path/Record mode/bundle identity in the version's ledger, matching existing binary hash, matching on-disk identity, valid signature/helper hashes, and that **selected** app be closed. Existing installed-baseline snapshot and independent-copy checks remain in place. It never launches, changes daemon lifecycle, or replaces production data.

Five pure path/ledger tests, Python syntax/help and diff checks passed. No bundle was staged or launched by this tooling change. The new helper build/sign/self-test/preflight and hash above were reported by the parent; this worker did not rerun them.

## Preserve the source-built daemon across native builds

The review configuration inherits `externalBin: binaries/permagentd`. Tauri's
build support copies that preserved installed sidecar into the shared
`debug/permagentd` path even for `--no-bundle`. This was observed replacing source
SHA `ea6263cd4f02075b3868ffea43cba584e2d937c7a8b2698f5e64cd7f1d2be89a`
with installed baseline SHA `fb9695d479c5786dfbb2dbe887577744b97910c0cdde7ac3aac883320a3a1810`.
The matching source artifact remained in `debug/deps/permagentd-16367fb0e75db594`.

`run_guarded_native.py` now preserves any existing canonical `debug/permagentd`
before invoking Tauri. It creates an independent APFS clone (ordinary copy as a
fallback), verifies both SHA-256 values, and retains it under
`native-daemon-preservation/<unique-id>/permagentd`. After the entire owned build
process group stops, it atomically restores and rehashes the original daemon if
Tauri changed or removed it. This runs on success, failed compilation, failed
launch, interruption, and deadline/disk abort. Restoration failure makes the
runner fail and records the recoverable backup location; symlinks, altered
backups, and a still-live build group are refused. No preexisting daemon is
explicitly recorded rather than misrepresented as a source build.

Each build summary records `daemon_artifact_preservation` and
`daemon_preservation_error`. Seven pure tests exercise independent inode copies,
unchanged/removed artifacts, unsafe paths/live groups/backup corruption, successful
and failed runner exits, launch/interruption cleanup, and restoration failure.
No build, daemon stop/start, or live artifact modification was needed for these tests.

The focused `run_record_native_tests.py` runner now uses the same preservation
helper, because native `cargo test` also executes Tauri's sidecar-copying build
script. It clears inherited review environment overrides, verifies the complete
owned process group has stopped before restoration, records restoration errors,
and preserves the original disk/deadline reason if cleanup itself fails. The
seven preservation tests cover both native entry points; the six existing guard
tests now include focused-native cleanup. All13 pass using temporary artifacts
and mocked processes; no real compiler or installed artifact is involved.
