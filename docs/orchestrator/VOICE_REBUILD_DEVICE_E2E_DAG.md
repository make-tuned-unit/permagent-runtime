# Voice rebuild + device E2E DAG (V6)

**2026-09-05 update:** the hosted-app-free iOS test execution gap is resolved:
130 tests passed, zero failed/skipped on a freshly built simulator test run.
See [completion ledger](END_TO_END_COMPLETION_LEDGER_2026-09-05.md#fresh-ios-regression-execution--passed).
Historical simulator errors below remain historical; live microphone, full app
and current-daemon/device journey gates are still not passed.

**Date:** 2026-09-04 (America/Halifax)  
**Status:** `blocked_at_device_boundary` / app build and install passed; simulator evidence failed after launch  
**Owner:** voice UX reliability fan-in  
**Device target:** iPhone 17 Pro simulator, iOS 26.5, `A82A2157-CFC5-41B9-91A7-611E5B75C7C9`

This receipt records the bounded V6 rebuild and simulator boundary attempt. It
does not claim a device or microphone pass. This child made only surgical
compile fixes in `VoiceView.swift`; daemon, provider, and production voice
behavior were not otherwise changed.

## Entry and freshness evidence

The current source and generated project were checked before the gates:

| Artifact | Observed mtime (AST) | Result |
|---|---:|---|
| `crates/goose-server/src/routes/voice.rs` | 2026-09-04 21:51:18 | current source |
| `ios/PermagentMobile/project.yml` | 2026-09-04 21:45:47 | source-of-truth project |
| `ios/PermagentMobile/PermagentMobile.xcodeproj/project.pbxproj` | 2026-09-04 21:48:18 before regeneration | regenerated below |
| `target/debug/permagentd` | 2026-09-04 16:45:00 | stale before this child |

Disk was 9.4 GiB free before the daemon build gate. The resource policy floor
is 8 GiB. No paid provider or network inference call was made.

## Gate V6.1 — regenerate project

Command:

```text
xcodegen generate --spec ios/PermagentMobile/project.yml --project ios/PermagentMobile/PermagentMobile.xcodeproj
```

Result: `passed`; XcodeGen wrote the project at
`ios/PermagentMobile/PermagentMobile.xcodeproj/PermagentMobile.xcodeproj`.
The generated project includes `VoiceProtocolTypes.swift` in the test target.

## Gate V6.2 — daemon source validation

Command:

```text
CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --lib --message-format=short
```

Result: `passed`, `Finished dev profile`, 57.11 seconds. This is a typecheck
of the current source and is not a fresh daemon executable.

The bounded executable build was then attempted once:

```text
CARGO_INCREMENTAL=0 cargo build -p permagent-daemon --bin permagentd --message-format=short
```

Result: `infrastructure_failed`: Rust/LLVM stopped with `IO failure on output
stream: No space left on device` while emitting the daemon library. The
pre-existing `target/debug/permagentd` remained at 2026-09-04 16:45:00 and was
not relabelled as current. The exact disposable binaries named by the resource
policy were removed after the failed build, restoring 8.9 GiB free. No second
unchanged daemon build was attempted.

## Gate V6.3 — iOS app/test build

The direct application-scheme build was bounded once:

```text
xcodebuild -project ios/PermagentMobile/PermagentMobile.xcodeproj \
  -scheme Permagent -sdk iphonesimulator -configuration Debug \
  -derivedDataPath /private/tmp/permagent-v4-derived-20260904 \
  CODE_SIGNING_ALLOWED=NO build -quiet
```

Result: `test_failed` at the build boundary: the embedded
`PermagentWatch` target was selected for the iOS simulator SDK and could not
resolve `WatchKit`. This is a project-target/build-selection defect, not an
iOS voice assertion. It was not retried unchanged.

The existing derived-data tree was reused to avoid a duplicate link-heavy
build:

```text
xcodebuild -project ios/PermagentMobile/PermagentMobile.xcodeproj \
  -scheme PermagentTests -sdk iphonesimulator -configuration Debug \
  -derivedDataPath /private/tmp/permagent-v4-derived-20260904 \
  CODE_SIGNING_ALLOWED=NO build-for-testing -quiet
```

Result: `passed` at approximately 21:56 AST for the hosted-app-free
`PermagentTests` target. This scheme intentionally does not depend on the
`Permagent` application target; the `Permagent.app` directory that Xcode
created is empty and is not an installable app. Built test products:

```text
/private/tmp/permagent-v4-derived-20260904/Build/Products/Debug-iphonesimulator/PermagentTests.xctest
/private/tmp/permagent-v4-derived-20260904/Build/Products/PermagentTests_iphonesimulator26.5-arm64-x86_64.xctestrun
```

This is deterministic test-bundle compile/link evidence only. It is neither an
installable app build nor simulator interaction evidence. Building the actual
`Permagent` app remains part of the blocked device gate because its generated
scheme embeds the watch companion and CoreSimulator is unavailable.

### Follow-up V6.3a — surgical app compile repair

The prior app failure was re-read from the Xcode activity log. The actual
errors were three `VoiceView.swift` type/API errors, not a WatchKit resolution
failure:

- `PendingReplySegment.segment` was declared non-optional even though the
  timing-validation path intentionally queues `nil` metadata.
- `UITextView.scrollRangeToVisible` was called with an unsupported `animated:`
  argument.
- `Brand.muted` does not exist; the established token is `Brand.textMuted`.

The first two were corrected with existing UIKit/type patterns; the
`allowBluetooth` deprecation remains an availability-safe warning because the
existing iOS 17 fallback is required.

Command:

```text
xcodebuild -project ios/PermagentMobile/PermagentMobile.xcodeproj \
  -scheme Permagent -configuration Debug \
  -destination 'platform=iOS Simulator,id=A82A2157-CFC5-41B9-91A7-611E5B75C7C9' \
  -derivedDataPath /private/tmp/permagent-v6-sim-build build
```

Result: `passed`. The build graph contained two targets: `Permagent` using
`iphonesimulator26.5` and `PermagentWatch` using `watchsimulator26.5`; the
watch product was then embedded under `Permagent.app/Watch`. No WatchKit
framework was resolved against the iOS simulator SDK. Installable artifact:

```text
/private/tmp/permagent-v6-sim-build/Build/Products/Debug-iphonesimulator/Permagent.app
```

## Gate V6.4 — simulator test/install boundary

The newly built app was installed and launched once:

```text
xcrun simctl install A82A2157-CFC5-41B9-91A7-611E5B75C7C9 \
  /private/tmp/permagent-v6-sim-build/Build/Products/Debug-iphonesimulator/Permagent.app
xcrun simctl launch A82A2157-CFC5-41B9-91A7-611E5B75C7C9 ai.permagent.ios
```

Both commands returned success; launch returned process identifier `76620`.
The subsequent single screenshot request did not execute because
`CoreSimulatorService` became invalid and refused the connection. No second
unchanged simulator request was attempted.

One bounded test-without-building attempt reused the built products:

```text
xcodebuild test-without-building \
  -xctestrun /private/tmp/permagent-v4-derived-20260904/Build/Products/PermagentTests_iphonesimulator26.5-arm64-x86_64.xctestrun \
  -destination 'platform=iOS Simulator,id=A82A2157-CFC5-41B9-91A7-611E5B75C7C9' \
  -resultBundlePath /private/tmp/permagent-v6-sim-test-20260904.xcresult \
  CODE_SIGNING_ALLOWED=NO -quiet
```

Result: `infrastructure_failed`, exit 70. At 22:05–22:06 AST, CoreSimulator
reported `connection invalid` / `connection refused`; the concrete iPhone 17
Pro destination was unavailable. `simctl list devices available` and the
Xcode destination list consequently exposed only the simulator placeholder,
not an operational device. The result bundle was written, but no test or UI
assertion executed. There was no install, launch, screenshot, or interaction
trace, and no simulator microphone evidence.

Earlier `xcodebuild -list` output did enumerate the target UUID and iOS 26.5,
but enumeration is not proof that CoreSimulator can boot or accept an app.

## Real-device boundary

No connected physical iPhone was available to this child (`xcrun devicectl
list devices` returned no device), and no microphone/audio capture was run.
The following remain explicitly deferred to a later fan-in gate with a live
device and freshly rebuilt daemon executable:

- speak a known phrase and verify partial/final transcript visibility;
- verify empty-STT and terminal-outcome recovery copy;
- verify reply text, active-word highlighting, multi-paragraph autoscroll,
  interruption reset, and accessibility labels;
- verify model/agent affordances and next-turn model switching;
- correlate socket epoch, playback, reconnect, and capture telemetry;
- capture privacy-safe simulator/device screenshots and latency evidence.

## Exit classification and bounded retry

V6 **does not pass**. The compile gate passed for the iOS app/test target and
the daemon typecheck passed, but the daemon executable freshness gate and the
simulator/device E2E gates are `infrastructure_failed`/`not_run`. The next
retry must materially change the environment (restore disk headroom and
restore CoreSimulator/physical-device availability); repeating the same
commands unchanged is prohibited by
`CODING_HARNESS_VERIFICATION_RESOURCE_POLICY.md`.

The master DAG should keep V6 active and keep V7 blocked behind this receipt.

## Subsequent materially changed simulator evidence

After the compile fixes and successful install/launch above, CoreSimulator
became responsive again. A new state probe showed the same iPhone 17 Pro
simulator as `Booted`, so one screenshot was taken without rebuilding:

```text
xcrun simctl io A82A2157-CFC5-41B9-91A7-611E5B75C7C9 \
  screenshot /private/tmp/permagent-v6-voice-current.png
```

Result: `passed`. The screenshot shows the freshly launched application at its
unpaired “Pair with your hub” screen. This proves the installable app renders,
but it is not voice-screen evidence: the simulator has no paired hub/session,
so transcript, reply highlighting, model controls, microphone, and playback
remain unexercised. V6 therefore remains active.
