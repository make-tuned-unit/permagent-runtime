# Scoped audio diagnosis — 2026-10-08

**Acceptance remains blocked.** Two real Chrome-scoped ScreenCaptureKit helper captures emitted only zero-amplitude PCM. The second used direct Web Audio playback, so WebRTC is not the sole cause. Lifecycle, source selection and final-tail delivery passed; usable audio capture did not. Neither run was the integrated Record UI/Tauri IPC flow.

## Evidence

Evidence root: `/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/record-qa-runtime/capture-evidence/`.

| Run | Helper SHA-256 | WAV duration / peak / RMS | Result |
| --- | --- | --- | --- |
| `chrome-272dc6f4655f4ce3925981ac4b38802e/capture.json` — WebRTC | `490f4e56bda9a67ce96b2a62aa441f1ae2fc099a07c531ec38a3bd0c30cfc701` | 45.00 s / 0 / 0; final tail 30.22 s / 0 / 0 | `capturePassed: false` |
| `chrome-44e18d46f3824db386f6d784ab9a06e4/capture.json` — direct playback control | `bde9c66f1259b244a293620b258ab9dacd15617d8a1c819be0e4de9402934db7` | 45.00 s / 0 / 0; final tail 30.04 s / 0 / 0 | `capturePassed: false` |

Both receipts have `preflightGranted`, `sourceVerified`, `ready`, `readersJoined` true and exit code 0. Both include a chunk emitted before stop and a final chunk after stop. These fields establish capture lifecycle/protocol, not speech or nonzero samples. The zero-amplitude acceptance check has not been weakened.

The direct-control binary is `/Users/j/Documents/dev/permagent-worktrees/.shared-target/world-production-20261006/permagent-audiocap-scope-r2`; its full SHA-256 was independently reread for this report and matches the receipt. Root reported that this helper compiled, was signed, and passed the complete `--self-test`, including owned-descendant scope and escape rejection. This diagnosis lane did not repeat builds or tests. `record-scoped-helper-verification.json` documents an older binary and must not be used as the scope-r2 build receipt.

Root's Computer Use observations: the WebRTC fixture connected with decoded audio energy 1.135 and 252 decoded video frames; the direct control displayed DIRECT AUDIO PLAYING, a running AudioContext, and successful fixture-file hash verification. These are playback/transport observations, not proof of nonzero OS output or captured speech.

## Scope and attribution

The helper was invoked with exact include bundle `com.google.Chrome`, excluded bundle `ai.permagent.record-review-20261007`, and `--require-existing-permission`. Its preflight marker was `PERMAGENT_AUDIOCAP_EXISTING_PERMISSION_V1`; its source marker was `PERMAGENT_AUDIOCAP_SCOPED_SOURCE_V1`. No permission broadening or all-application fallback was used.

The scope-r2 predicate accepts the exact selected root plus descendants only when PID ancestry and canonical executable ownership under that root's `.app/Contents` both succeed. It rejects unrelated ancestry, missing roots/metadata, cycles, excluded identities, path-prefix collisions, outside intermediaries and executable symlink escapes.

**That change did not add a selected process on this host.** Root ran `--list-source-scope` after compilation: ScreenCaptureKit's actual inventory selected only Chrome PID 726, bundle `com.google.Chrome`; audio utility PID 66550 was not exposed as a candidate. The direct capture receipt independently logs only the selected PID 726. The diagnostic mode performs existing-permission preflight and exits before creating any SCStream.

Separate read-only process metadata established:

- Chrome root PID 726, bundle `com.google.Chrome`.
- Audio utility PID 66550, parent 726, `audio.mojom.AudioService`, executable inside Chrome's bundled `Google Chrome Helper.app`, bundle `com.google.Chrome.helper`.
- CoreAudio exposes the root and audio utility as separate process objects (118 and 119 at observation time). A third helper, PID 66551, was the video-capture service and appeared as object 120.

This supports investigating audio-service attribution, but does **not** establish how ScreenCaptureKit internally attributes child audio. The descendant change cannot resolve an absent SCK candidate, and this report does not claim that it fixed capture.

`excludesCurrentProcessAudio = true` is documented in the installed SDK as excluding the current process; here that is the capture helper. The helper's parent PID is the QA harness, not Chrome. There is no evidence that this flag excludes Chrome's audio utility. Changing it is not presently a grounded fix. The direct receipt's “excluded applications: 0” reports the independently computed exclusion list; the active filter is still the explicit Chrome include list.

## Output-route check

At `2026-10-08T03:07:34Z`, read-only CoreAudio property calls found both default output and default system output pointing to device 96, **External Headphones**, built-in transport, alive=1. Master output volume scalar was **0.312542587518692**, with **mute=0**. Per-channel volume/mute controls were unavailable; the master controls returned valid values.

Full receipt: [RECORD_AUDIO_DEVICE_READONLY_2026-10-08.json](RECORD_AUDIO_DEVICE_READONLY_2026-10-08.json). No setters, taps, recording, permissions, UI or browser-profile access were used. The fixture had finished, so device/process running-output flags were 0. This rules out zero master volume or mute **at observation time**, not during the earlier captures.

The fixture source defaults far-side output to enabled at 0.65 volume. In WebRTC mode it explicitly plays received audio elements; in direct mode a GainNode connects to `AudioContext.destination`. Those source defaults and a running context do not prove current browser site/tab sound state or acoustic output. Browser sound-settings inspection was blocked by browser security policy; no alternate API, profile-file, or native-UI workaround was attempted. Site/tab sound state remains unverified.

## Bounded next work

No additional capture is justified by the current evidence alone. Do not broaden to all-system audio to make a test pass. Keep native Record acceptance blocked until an authorized, app-scoped capture contains actual far-side speech and passes through Record assembly/save.

A future isolated implementation could evaluate Apple's process-specific CoreAudio tap API: resolve the selected app's currently owned audio process to its CoreAudio process object, then create a `CATapDescription` for only those explicit objects via `AudioHardwareCreateProcessTap`. Apple documents capture of a process or group of processes. This is a separate implementation and permission/lifecycle validation task, not an implemented fallback or guaranteed fix. Process/object IDs are ephemeral; an implementation would need ownership revalidation, fail-closed process-exit handling, explicit permission behavior, cleanup and nonzero-audio acceptance. [Apple: Capturing system audio with Core Audio taps](https://developer.apple.com/documentation/coreaudio/capturing-system-audio-with-core-audio-taps?changes=late_1_5_5).

Final diagnosis: **silent app-scoped output reproduced; exact cause unresolved**. No source changes, builds, capture, or UI operations were performed in this final read-only diagnosis pass. Only this report and its read-only device receipt were added.
