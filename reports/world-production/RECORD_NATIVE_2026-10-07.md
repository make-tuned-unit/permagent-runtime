# Record native capture repair — 2026-10-07

**Latest state:** scoped Swift helper compiled, signed and passed all five fixture groups. Native Rust application-list/scope additions await the parent’s cargo gate. Use the new scoped helper/hash in the final section; earlier lifecycle-only hashes are historical.

## Scope and current evidence

Changed capture source `ui/desktop/src-tauri/src/system_audio.rs` and `ui/desktop/src-tauri/audiocap/main.swift`, the explicitly authorized optional helper staging override, a guarded native test runner, and this report. No UI control, capture permission request, live recording, installed application mutation, daemon change or commit. Rust source was formatted and compiled successfully in the granted test slot; all 3 focused native tests passed. The running review shell has not yet been rebuilt/staged with these changes.

The isolated Swift helper compiled successfully with the installed Xcode SDK, was ad-hoc signed and passed `codesign --verify --strict`. Running `--self-test` (which exits before ScreenCaptureKit/TCC) returned exit 0:

```
PASS planar stereo, interleaved stereo, mismatched channel rejection, CoreMedia undersized-list regression
```

The CoreMedia fixture constructs a real planar stereo CMSampleBuffer. A fixed `MemoryLayout<AudioBufferList>.size` output returns `kCMSampleBufferError_ArrayTooSmall`; a queried-size allocation returns both channels and the expected downmix. This establishes the old allocation defect for that layout; actual live ScreenCaptureKit layout and end-to-end speech remain acceptance work. An early fixture failed because its synthetic sample lacked timing metadata; the corrected fixture includes sample timing and passes in both diagnostic and optimized builds.

## Changes

- Start holds the native lifecycle mutex through child spawn and reader registration, rejects concurrent active starts, and reaps an already-exited helper before a later start. Duration is finite and bounded to 1–300 seconds.
- Native creates a fresh private directory (0700 on Unix) under an approved system-temp parent. Existing directories are rejected. Only numeric `sysaudio_NNNN.wav` paths actually emitted by this owned child can be read. Reads reject symlinks, use O_NOFOLLOW and enforce a 10 MB limit. Successfully read files are deleted; empty inactive directories are removed. A failed transcription still has no native audio retry because the existing read-once contract is retained.
- Stop keeps ownership while signaling the helper, waits/reaps it, and joins stdout/stderr readers. Forced timeout returns an explicit incomplete-tail error. Drop kills/reaps only the owned child; helper independently stops when its parent exits.
- Stop also returns unread paths as an explicit final-tail delivery receipt (contract below). Native event emission alone cannot prove the webview has run the callback.
- Swift queries and allocates the full variable-sized AudioBufferList. Downmix validates supported Float32 PCM, channel sizes, finite samples, and handles planar/interleaved layouts without out-of-bounds reads.
- Swift handles termination on a separate lifecycle queue. The previous handlers used `.main` while the main thread was blocked by `sem.wait()`. Stop now stops ScreenCaptureKit delivery, drains the serial sample queue and writes the final partial chunk once; sample writes and chunk numbering share that queue.
- Rust passes parent PID and bundle identifier. ScreenCaptureKit excludes matching parent applications, while `excludesCurrentProcessAudio` excludes the helper itself. Parent-app WebKit subprocess attribution still needs an actual Permagent TTS exclusion test; no complete echo-isolation claim is made. A practice call hosted inside Permagent may be excluded, so use the separate controlled call wrapper.

## Frontend IPC contract

Existing input arguments remain unchanged:

```
start_system_audio({ outDir, chunkSeconds }) -> null
read_audio_chunk({ path }) -> number[]
stop_system_audio() -> string[]
```

`stop_system_audio` returns this capture's completed **unread** paths, sorted by path, after helper exit and reader joins. An inactive stop returns `[]`. Paths already read are omitted; paths whose emitted event has queued but not yet read may appear in both event and result. Frontend must route event/result through one per-recording dedup-by-path handler, retaining its immutable recording identity, **before** removing listeners and awaiting the transcription queue. Older native builds return null/undefined; normalize those to `[]`. Ignoring the extra return remains compatible but does not close the JS dispatch race. A forced timeout returns an error rather than falsely acknowledging the tail.

Rust additionally injects `AppHandle` into read_audio_chunk; the JS argument shape does not change. Native start now rejects a second active owner instead of claiming successful ownership of the first recording.

## Build and stage prerequisites

Isolated helper output (not copied over the source-tree baseline helper):

```
/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-record-review
```

- Signed helper SHA-256: `ab8d4a43f57c146bfac539b123e09e153292b2271f80f3b6d0902c66f355c456`
- Swift source SHA-256: `c18251cca2f949061587268227891088fd8e18f25f2e3502bce2d7889adfb23d`
- Architecture: arm64 Mach-O.
- Free disk after compile: approximately 37.7 GiB, above the 20 GiB floor.

Compile command, run only in the coordinated slot:

```
xcrun swiftc -O -o <review-target>/permagent-audiocap-record-review \
  ui/desktop/src-tauri/audiocap/main.swift \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia
codesign --force --sign - <review-target>/permagent-audiocap-record-review
<review-target>/permagent-audiocap-record-review --self-test
codesign --verify --strict <review-target>/permagent-audiocap-record-review
```

The ordinary `ui/desktop/scripts/build-audiocap.sh` writes `src-tauri/audiocap/permagent-audiocap`. It was deliberately not run here because review staging and prerequisites currently reuse the installed helper. Tauri's resource map places that file at `Contents/Resources/permagent-audiocap`. Native lookup gives `Contents/MacOS/permagent-audiocap` precedence if present, then the resource location; inspect the review bundle and do not leave a stale higher-priority copy.

Parent staging must explicitly accept a reviewed helper override: close the review app, retain an independent backup and hash of its previous helper, copy the isolated signed helper into the resolved review-only helper path using an independent inode, re-sign the review bundle, verify deep/strict signature and resolved helper hash, and record the override in the packaging ledger. Preserve the installed baseline and its prerequisite hashes. Merely rebuilding Tauri or running the existing stage script does **not** select this new Swift helper. Do not quietly relax the baseline prerequisite manifest. The source-tree helper also remains old for dev fallback until an explicit normal packaging build replaces it.

## Focused native tests — passed

Rust tests in system_audio.rs:

1. Numeric-only chunk names reject broad basename matches.
2. A fresh capture directory is exclusive; foreign parents and symlink chunks are rejected.
3. A deterministic shell helper emits a tail on SIGTERM; stop must wait for a delayed stdout reader to observe it, and reap the child. This uses no audio/capture permissions.

The granted native test slot ran `cargo test --manifest-path ui/desktop/src-tauri/Cargo.toml --locked -j 1 system_audio::tests -- --nocapture` with review TAURI_CONFIG, jobs=1, debug=0 and incremental=0. All 3 tests passed, 72 unrelated tests were filtered out. Compilation took 41.02 seconds; guarded run 42.1 seconds with minimum 35.993 GiB free and exit 0. Logs: `record-native-tests-20261007-214248.log` and its JSON sibling. The guard starts only above 25 GiB, stops its own process group below 20 GiB, and never signals global cargo/rustc processes. Rebuild the actual review shell with frontend stop-result handling next. Live acceptance must still verify: far-side audio before/after the 45-second boundary; final tail once; mic denial stops the helper; double-start/discard/restart ownership; parent-app quit kills capture; privacy prompt behavior; Permagent TTS excluded and external call audio included; read paths stay confined; helper/note/transcript evidence recorded separately from fixture ground truth.


## Optional helper staging override — implemented, not executed

`stage_native_review.py` now accepts a paired explicit helper/hash override and retains the old behavior when none is supplied. It validates the signed arm64 file and expected hash before any mutation, preserves prior helper candidates with the shell/Info.plist/signature backup, checks previous override evidence on refresh, independently copies both resource/MacOS candidates as applicable, and records the resolved helper path and final hash in the packaging ledger. After recursive signing it restores the exact signed helper and reseals the outer app, so the final helper must match the explicit expected hash. A later shell-only refresh preserves the previously reviewed helper from the independent backup. Installed-baseline snapshots and running-review checks remain intact.

After parent closes the review app and verifies the new native executable/assets, use:

```
python3 reports/world-production/stage_native_review.py --refresh \
  --audio-helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-record-review \
  --audio-helper-sha256 ab8d4a43f57c146bfac539b123e09e153292b2271f80f3b6d0902c66f355c456
```

Validation performed without staging: Python syntax compilation, CLI help, missing-paired-hash rejection (exit 2), and wrong-hash rejection (exit 1). The review app remained running and no bundle was changed. Actual copy/sign/hash verification awaits the coordinated stage operation.

## Follow-up: application-scoped capture (Swift verified; native Rust gate pending)

Live QA found external headphones were the default output and a C922 webcam microphone was the default input. Ambient microphone pickup is not a valid synthetic near-side fixture. The parent stopped that test; no global routing changes are needed for the following safe far-side path.

Added optional `includeAppBundleId` to `start_system_audio`. Omitted/null preserves all-system capture. A supplied value is validated as an exact bundle identifier and must differ from the recording app. Rust passes `--include-bundle-id` to the helper; Swift requires at least one exact running match in `SCShareableContent`, rejects the parent PID/bundle, and constructs `SCContentFilter(display:including:exceptingWindows:)`. A missing source reports `ERROR source` and exits before capture. It never silently falls back to all applications.

A mixed-version shell/helper is particularly dangerous here: older helpers ignore unknown flags. Before spawning with explicit scope, Rust therefore checks the trusted helper binary for the embedded capability marker `PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1`. Old helpers fail closed with an update/rebuild error. The marker is emitted in a SOURCE diagnostic by the new helper. This is a compatibility check for a trusted bundled executable, not code-signature verification or a security boundary against a tampered helper.

New IPC:

```
list_system_audio_apps() -> [{ bundleId: string, name: string }]
start_system_audio({ outDir, chunkSeconds, includeAppBundleId?: string | null }) -> null
```

The list comes from `NSWorkspace.runningApplications` on the main thread, filtered to regular running apps with bundle identities, excluding the current app, sorted by display name and deduplicated by bundle ID. It does not invoke the helper or ScreenCaptureKit and does not request capture permission. The list represents selectable running apps, not proof that an app currently emits audio; the actual source is checked again against ScreenCaptureKit at Start. `main.rs` adds only the new command registration; no dependency/lockfile changes.

For controlled far-side QA choose `ai.permagent.record-practice-20261007` (the dedicated practice wrapper). Keep microphone capture disabled if the frontend supports this mode; otherwise physically silent microphone capture is still ambient capture and must be disclosed rather than labeled a clean isolated test. No system microphone/output routing change is part of this implementation.

Additional pure tests cover invalid/wildcard/whitespace bundle identities and own-parent exclusion in Rust; Swift self-test covers exact match, prefix mismatch, same-PID and same-bundle exclusions. **Scoped Swift changes are now compiled and tested; native Rust IPC/source-list additions remain uncompiled.** Prior hashes/test results above refer to the previous lifecycle/stereo build; do not stage that old helper expecting source scoping. Current verified scoped helper follows.


### Scoped helper verification and source freeze

The granted short Swift slot compiled the helper with `swiftc -O` and ad-hoc signing; strict signature verification and `--self-test` both returned exit 0. The initial compiler run identified the imported Swift API spelling `including:` (the Objective-C selector uses `includingApplications:`); source was corrected before the successful build. No ScreenCaptureKit permission request or capture was performed by these checks.

```
PASS planar stereo, interleaved stereo, mismatched channel rejection, CoreMedia undersized-list regression, exact application scope
```

- Current reviewed helper: `/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-scoped-review`
- Signed helper SHA-256: `0318ce2682c8a1b917ba91e36c79911ee881c6d591a23298df014d0ed3e28fa7`
- Swift source SHA-256: `66975f1ac1bed9fe7bbf4b6b67bfcd1000bff0780e3497ede9fbf0d1f678e50e`
- Scoped protocol marker: present in this helper, absent in the historical lifecycle-only helper. Explicit scope therefore fails closed when that old helper is used.
- NSWorkspace read-only fixture readiness: `ai.permagent.record-practice-20261007`, display name `Permagent Record Practice`, PID 83059, regular activation policy, not terminated. This checks the same public app enumeration primitive used by the picker; it does not establish that the as-yet-uncompiled Rust IPC has run.

Current staging command, after parent native build and app closure:

```
python3 reports/world-production/stage_native_review.py --refresh \
  --audio-helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-scoped-review \
  --audio-helper-sha256 0318ce2682c8a1b917ba91e36c79911ee881c6d591a23298df014d0ed3e28fa7
```

Native and Swift source are frozen for the parent’s native compile. No cargo invocation, bundle staging or app launch occurred in this follow-up slot.
