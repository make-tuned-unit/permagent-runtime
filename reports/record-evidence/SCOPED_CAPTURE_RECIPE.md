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
