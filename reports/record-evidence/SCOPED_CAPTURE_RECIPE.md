# Chrome-scoped live helper capture — parent execution only

This path can establish actual ScreenCaptureKit audio capture, helper chunk protocol/final flush, and subsequent local Whisper from those captured WAVs. It does **not** exercise Record UI, Tauri `start/read/stop_system_audio` IPC, microphone/duplex capture, autosave/project selection, or meeting triage. Browser CUA and existing HTTP acceptance remain separate evidence.

No capture, helper build, permission check, device change, inference or UI interaction was performed by this worker while preparing this path.

## Existing-permission guard requires a newly reviewed helper

The previously compiled scoped helper ignores unknown arguments. Do not pass new preflight flags to that old helper: it would proceed into capture. The runner therefore requires both embedded capability markers and an explicit exact SHA-256 before executing anything:

- `PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1`
- `PERMAGENT_AUDIOCAP_EXISTING_PERMISSION_V1` (new)

The source now supports `--preflight-only`, which calls public `CGPreflightScreenCaptureAccess()` and exits before ScreenCaptureKit. `--require-existing-permission` repeats that same check before capture. The installed SDK's `CoreGraphics.framework/Headers/CGWindow.h` defines the former as checking whether the current process already has capture access; its separate request function can prompt and is **never called** here. This check runs in the helper identity itself, avoiding a Python/Terminal grant being mistaken for its grant.

If access is absent, the helper returns `ERROR permission`/exit2. The runner fails before capture, without attempting a permission request, grant, reset, retry, alternate API or unscoped fallback. A conservative false negative is acceptable. Screen Recording access granted to another app is not proof this helper has it. Existing full screen-capture access is required even if the OS supports more narrowly described audio permissions.

The worker changed source only. Swift source SHA-256 is `3f3e9e8cf6d1d6e94beec94795ea76de8a69725c467cee227be3af3e1e24aa65`. When the parent releases a short compiler slot, build/sign to a **new** owned path; preserve the installed/previous helpers:

```sh
swiftc -O ui/desktop/src-tauri/audiocap/main.swift \
  -o /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-preflight-review \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia -framework CoreGraphics
codesign --force --sign - /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-preflight-review
codesign --verify --strict /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-preflight-review
```

Then run its noncapturing `--self-test`, record the signed helper hash and confirm both markers. No source-bundled helper/app staging is required for this separate CLI experiment. Passing the new helper's `--preflight-only` cannot request access; if it fails, stop this path and record that limitation.

## Source scope and CUA sequence

The source is fixed to exact bundle ID `com.google.Chrome`. Parent has observed Chrome tabs as task-only; recheck that condition immediately before execution and ensure other task tabs are silent. This is app-wide capture, not tab isolation. Capture must stop if unrelated/private Chrome audio becomes possible. Nothing opens a microphone, changes a device, sets the system default input/output, installs loopback drivers or plays audio automatically.

The helper's implementation is ScreenCaptureKit, **not a CoreAudio PID tap**. It asks for `SCShareableContent`, requires an available display and a matching application, then uses the exact-application inclusion filter. A missing source produces `ERROR source`; it never falls back to all apps. CUA's native-window lookup failure alone does not prove this API unavailable. A locked/no-display/session permission problem can still prevent capture; report the real failure rather than infer success.

With Cargo/inference pressure cleared, daemon3002 prepared and the local fixture ready at `http://127.0.0.1:5489/`, run:

```sh
python3 reports/record-evidence/scoped_capture.py capture --execute \
  --helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-preflight-review \
  --sha256 <new-reviewed-signed-helper-sha256> --seconds 75
```

The runner validates the isolated artifact manifest, helper path/hash/signature/markers and existing permission. Capture starts in one owned process group with45-second chunks and exact Chrome scope. It waits at most30seconds for matching `SOURCE` then `READY`. Only after its READY JSON should the parent use CUA to start **Far-side55s → WebRTC practice call**, ideally within10seconds. The fixture sends no microphone/camera audio; it plays the synthetic remote voice via decoded local WebRTC. The positive commitment starts at3seconds; cancellation/negation follows; the final venue route commitment starts49seconds and ends near52.8seconds.

The runner captures75seconds after READY, then sends SIGTERM only to its owned group and waits for exit/stdout EOF, preserving the final partial chunk. Ctrl-C/termination also performs owned cleanup. Because the final cue is after49seconds, it should land after the45-second full chunk and remain in the stop-flushed partial chunk. Failure/timeout does not prompt for broader permission or recapture automatically.

## Evidence and separate transcription

Each run creates private `<isolated-root>/capture-evidence/chrome-<uuid>/`, outside the public fixture server's directory. `capture.json` records exact helper/hash/command, source/READY diagnostics, emitted paths/times, stop timing, reader joins, exit code, file hashes, sample format, duration and signal amplitude. It validates numeric canonical regular WAV paths, rejects symlinks/duplicates/unemitted files and caps each file at10MiB. It requires nonzero samples and an emitted post-stop partial chunk before `capturePassed=true`.

Retain these WAVs as actual captured evidence. Captions/source WAVs are not substituted for capture. `capturePassed` establishes protocol/signal checks, not intelligible speech or source content; parent must correlate the CUA fixture run/timestamps and then request transcription separately:

```sh
python3 reports/record-evidence/scoped_capture.py transcribe --execute \
  --run-directory <canonical-run-directory-from-READY>
```

Transcription rechecks helper/model/chunk hashes, calls only isolated3002 with the bearer in memory and saves private observed-text evidence. Assertions require accessibility-checklist commitment, cancelled purchase, no purchase task, and the final venue step-free-route cue specifically in the stop-flushed tail. No transcript is auto-sent to a project/Orchestrator by this helper. The full integrated Record flow remains unpassed until native UI/IPC can actually be driven.

Validation performed: four pure Python/source contract tests, syntax, dry-run and diff checks. These test generated temporary PCM files/path rejection and the new source's nonprompting branch ordering; they do not compile Swift or prove live ScreenCaptureKit behavior. Native helper build/self-test/preflight/capture are parent-owned pending steps.

## 2026-10-08: distinguish silent input from PCM conversion loss

The two earlier app-scoped captures each produced a45-second chunk and a roughly
30-second final tail with zero PCM peak/RMS. Their successful source/READY/stop
protocol does not establish usable captured audio. The next helper adds
`--pcm-diagnostics`, off by default and accepted only with exact Chrome inclusion
and `--require-existing-permission`. It reports the audio format once and
cumulative amplitude statistics every5seconds/final stop: raw channel samples,
downmix, resampled Float, and quantized Int16. These are counts/peak/RMS only;
no sample values, content, URLs, or window titles are logged. Default capture
retains its existing behavior and pays no per-sample diagnostics work.

The capture runner accepts the same optional flag, rejects helpers without its
capability marker, and stores `pcmFormat`, `pcmDiagnostics`, and `pcmDiagnosis`.
The diagnosis distinguishes incoming silence, stereo cancellation, resampling
loss, quantization loss, and nonzero data reaching the WAV conversion. Raw channel
zeros establish that the supplied SCK PCM is silent; they do not distinguish
source playback, routing, session state, filtering, or TCC causes. Nonzero raw
channels with zero downmix specifically expose phase cancellation. Extremely low
nonzero float input can quantize to zero. Normal speech should not disappear in
the existing straightforward Float32 averaging; the actual raw metrics must
settle this rather than assuming the converter or macOS is responsible.

After the parent grants a short compiler slot with resource limits satisfied,
use a new output; do not overwrite staged/installed or prior reviewed helpers:

```sh
swiftc -O ui/desktop/src-tauri/audiocap/main.swift \
  -o /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review \
  -framework ScreenCaptureKit -framework AVFoundation -framework CoreMedia -framework CoreGraphics -framework AppKit
codesign --force --sign - /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review
codesign --verify --strict /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review
/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review --self-test
shasum -a 256 /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review
```

The self-test adds aggregate-amplitude silence/signal/anti-phase/quantization
fixtures. It does not prove real capture. Only after its existing-permission
preflight and parent capture authorization, rerun the same CUA fixture sequence:

```sh
python3 reports/record-evidence/scoped_capture.py capture --execute \
  --helper /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-pcm-review \
  --sha256 <exact-new-signed-helper-sha256> --seconds 75 --pcm-diagnostics
```

No unscoped capture, alternate browser, browser security workaround, device
configuration change, microphone input, or new permission request is part of
this diagnostic sequence. The runner still requires the isolated artifact
manifest; do not weaken it to run while the daemon/artifact lane is inconsistent.

## Read-only CoreAudio process routing probe and tap feasibility

`audio-diagnostics/chrome_audio_process_probe.swift` queries only an explicitly
selected Chrome root PID and its explicitly selected AudioService PID. It checks
exact root bundle ID, executable ownership under the Chrome bundle, full ancestry,
exact Chromium AudioService command tag, and stable PID start times before/after.
It never prints command-line arguments, app paths, tabs, or URLs. HAL queries
translate that PID to its audio process object, confirm the PID, and report
`isRunningOutput` and the process's output-device IDs/alive/running/rate/mute/volume
when supported. No tap, aggregate, capture, permission request, property write, or
routing change occurs. Missing/changed process identity fails closed.

After parent compiler authorization:

```sh
swiftc -O reports/record-evidence/audio-diagnostics/chrome_audio_process_probe.swift \
  -o /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/chrome-audio-process-probe \
  -framework CoreAudio -framework AppKit
codesign --force --sign - /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/chrome-audio-process-probe
codesign --verify --strict /Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/chrome-audio-process-probe
```

Root selects the current verified task-only Chrome root and AudioService PIDs.
While root's CUA fixture plays, the explicit read-only invocation is:

```sh
/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/chrome-audio-process-probe \
  --chrome-pid <observed-chrome-root-pid> --audio-pid <observed-chrome-audioservice-pid>
```

An active WebRTC graph with `isRunningOutput=0` or no output devices points toward
rendering/output routing before capture. Active HAL output combined with SCK raw
zeros points toward filtering/session/permission rather than our converter; it
is not by itself proof which of those is responsible.

A possible later CoreAudio capture would use
`CATapDescription(stereoMixdownOfProcesses: [theExactProcessObject])`,
`isExclusive=false`, `isPrivate=true`, `muteBehavior=.unmuted`, a private owned
aggregate containing only that tap, and an IOProc reading only that aggregate.
No global tap initializer, process restore, PID substitution, default-device
write, physical-device mute, or all-system fallback is appropriate. On exit or
selected-process identity change, stop/remove the owned IOProc, destroy its
aggregate, and destroy its tap. The first experiment would log only amplitude
and format before authorizing a bounded private WAV capture.

**This tap experiment is not authorized to start under the present
existing-permission-only constraint.** Apple's primary tap documentation says
initial recording from the aggregate requests system-audio recording permission
and requires `NSAudioCaptureUsageDescription`. The installed public CoreAudio
headers expose process/tap APIs but no tap-specific nonprompting authorization
preflight. `CGPreflightScreenCaptureAccess` is documented for screen capture;
we cannot treat that as proof that a new tap helper identity has the needed grant.
Do not test this by starting a tap, querying private TCC APIs, changing grants, or
reading a permission database. The safe next live measurement is the instrumented
already-authorized SCK path, plus the read-only HAL probe.

Primary references: [Apple CoreAudio tap sample](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps),
[CATapDescription](https://developer.apple.com/documentation/coreaudio/catapdescription),
and [audio capture usage description](https://developer.apple.com/documentation/bundleresources/information-property-list/nsaudiocaptureusagedescription).
The local SDK `AudioHardware.h` documents PID translation (unknown PID returns
`kAudioObjectUnknown`), process output activity, and output-device scoping;
`CATapDescription.h` documents inclusion/exclusion/private/unmuted semantics.
Seven Python/source contract tests pass. New Swift source/fixtures remain
uncompiled and unexecuted pending the parent resource/build slot.
