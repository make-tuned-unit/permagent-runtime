# Permagent companion C0 baseline and ownership receipt

## Root gate reconciliation — 2026-09-05

**C0 audit gate passed; product qualification has not passed.** The original
assessment below is retained as historical evidence. The master defines C0's
exit as ownership/unresolved evidence mapped and a frozen baseline/platform
journey matrix. This receipt supplies that matrix, including explicit unknowns;
the current `END_TO_END_COMPLETION_LEDGER_2026-09-05.md` supplies active path
ownership and execution order. A clean worktree, finished C7 device journeys,
and all downstream measurements are not prerequisites in C0's declared gate.
Requiring them here would prevent the work intended to produce them.

All unknown authorship is preserved as pre-existing shared work. Workers own
explicit disjoint paths; root reviews integration. Fresh evidence in the ledger
supersedes only its matching historical entries: 130 simulator tests passed and
the app/watch bundle built and launched to unpaired onboarding. No live voice,
daemon route, physical watch, permission journey or cost qualification is
promoted by this audit decision. C1 is ready and remains incomplete until its
combined fault fixtures and inherited B5/B6 gates pass.

Date: 2026-09-05 (America/Halifax)
Status: **not promoted** — inventory and journey matrix recorded; baseline,
runtime, and device evidence remain incomplete.
Scope: read-only inspection and one deterministic Python fixture test. No
provider, secret, private transcript, GUI/device session, database write, or
production edit was performed.

## C0 decision

The companion master is structurally ready for C1, but C0's exit gate is not
met. The repository is not a clean attribution boundary, the current active
goal/card identity is not exposed by the read-only local checks, and there is
no fresh source-correlated daemon executable, live multi-surface runtime
journey, or physical iPhone/watch evidence. Existing compile and focused-test
receipts remain evidence for their exact scope only; they are not promoted into
a current end-to-end pass.

The matrix below is frozen as a baseline with `unknown`/`not run` entries. This
is intentional: missing measurements are unrated, not zero, and an existing
status label is not treated as proof.

## Source identity and ownership boundary

Read-only commands:

```text
git rev-parse HEAD
737c96968b1b742a8b161c63ed778c979298a706

git status --branch --short
## dogfood/runtime-fixes-20260831...origin/main [ahead 143, behind 35]

git log -1 --format='%H%n%ad%n%s' --date=iso-strict
737c96968b1b742a8b161c63ed778c979298a706
2026-09-02T00:20:17-03:00
Back off the analytics drain poller on quiet sites

git status --porcelain=v1 | awk 'BEGIN{m=0;u=0} /^\?\?/ {u++; next} {m++} END {print "modified=" m " untracked=" u}'
modified=159 untracked=85

git diff --check
exit=0
```

The 159 modified and 85 untracked paths include active Rust daemon/CLI/core
changes, iOS/Watch sources and generated project files, Command Center files,
and many orchestrator receipts. Their authorship and sequencing are not
recoverable from Git status alone. They are preserved as pre-existing shared
work; this receipt does not claim or reformat them. The C0 receipt itself is
the only new artifact owned by this child.

The active goal/card identity is **not observable** from the repository-only
read-only checks. `PERMAGENT_COMPANION_MASTER_PROGRAM_DAG.yaml` reports
`c0_evidence_and_ownership: active`, with C1–C7 planned. The parent-reported
topology validation passed; this child did not rerun a dispatch or runtime
validator and does not treat topology validation as product execution.

## Inherited program state

| Existing authority | Observed state | C0 treatment |
|---|---|---|
| Companion master `PERMAGENT_COMPANION_MASTER_PROGRAM_DAG.yaml` | C0 active; C1–C7 planned; C1–C6 point to the companion child anchors | Preserved; no status mutation. |
| Coding harness B5 | B5.3 receipt says implemented, promotion pending focused runtime capacity; B5.4/B5.5 remain pending in the task-budget DAG | Consumed as evidence only; no B5 implementation or duplicate projection work. |
| Coding harness B6 | Read-only adversarial gap audit; not passed. Integrated restart, duplicate-child, mixed-class, and selection/claim race proof remain partial | Consumed as evidence only; no B6 fixture duplication. |
| Voice V6 | `blocked_at_device_boundary`; source/compile/app-install evidence exists, but simulator test/device voice evidence is absent | C2/C7 must inherit this debt; no stale binary is accepted. |

### B5.3 receipt evidence

`docs/orchestrator/CODING_HARNESS_B5_3_CLI_ANNOUNCEMENT_RECEIPT.md` records:

```text
CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --lib   passed
CARGO_INCREMENTAL=0 cargo check -p permagent-cli --lib      passed
git diff --check -- <three B5.3 files>                      passed
```

Its status is **implemented; promotion gate pending focused runtime test
capacity**. The focused event/daemon route runtime tests were not promoted
because Cargo processes retained the shared build lock. This is compile and
source-contract evidence, not a daemon runtime pass.

The B6 audit is explicitly **read-only / not passed**. It identifies seam-level
proof but missing one integrated no-model sequence covering restart/reconcile,
duplicate child completion, mixed billing classes, and the atomic selection vs
claim race. C1 must reference those gaps rather than restating B6 as complete.

## Confirmed source boundaries

These are source boundaries found in the current tree; they are not claims that
all paths currently pass.

| Concern | Confirmed paths and boundary | C0 note |
|---|---|---|
| Existing coordinator/worker lifecycle | `crates/goose/src/agents/platform_extensions/orchestrator.rs`, `goal_engine.rs`, `summon.rs`, `dispatch_brief.rs`, `supervised_cli.rs`, `terminal_supervision.rs` | Internal/external worker engines, worktree/evidence capture, supervision, and handoff live here. No second coordinator authorized. |
| Permission and write scope | `dispatch_scope.rs`, `write_scope.rs`, `developer/edit.rs`, `developer/shell.rs`, `developer/verify.rs`, `execution_receipt.rs`, `publish_sequence.rs` | File edits, shell/computer effects, verification, receipts, and landing are separate seams. Exact cross-seam runtime order remains unverified. |
| App/computer tools | `app_conductor.rs`, `browser.rs`, `code_execution.rs`; `crates/goose-mcp/src/computercontroller/platform/mod.rs`, `macos.rs` | macOS automation is a platform implementation behind `SystemAutomation`; capability/OS permission journeys are not run in C0. |
| Existing scheduler | `crates/goose/src/scheduler.rs`, `scheduler_trait.rs` | Scheduler remains the only scheduled-work controller; no new scheduler is proposed. |
| Spectral/session memory and recognition | `crates/goose/src/recognition.rs`, `recognition_sink.rs`, `recognition_consent.rs`, `session/spectral_schema.rs`, `docs/architecture/SPECTRAL_INTEGRATION.md` | Recognition query mode and durable recall instrumentation have source boundaries. Ambient `StreamTracker` wiring is documented as not implemented; no ambient pass is claimed. |
| Budget/cost authority | `session/budget_projection.rs`, `cost_router/{budget,reservation,hold_done,cheap,tier,fallback,escalation}.rs`, `routes/coding_session.rs`, CLI `session/spend_announce.rs` | Spectral/session ledger and reservations remain authoritative; B5 promotion is pending. |
| Backend voice/state | `crates/goose-server/src/routes/voice.rs`, `routes/events.rs`, `routes/config_management.rs`, `routes/coding_session.rs`, `routes/runs.rs`, `routes/status.rs` | WebSocket/HTTP/SSE route boundaries are confirmed in source; cross-platform convergence is not runtime-proven. |
| macOS/desktop Command Center | `ui/command-center/src/ChatApp.tsx`, `lib/voiceHandoff.ts`, `components/voice/*`, `components/build/*`, `lib/costMeter.ts`, `ui/desktop/src-tauri/*` | UI package and Tauri desktop sources exist. Current test execution and installed desktop journey are absent from C0 evidence. |
| iOS | `ios/PermagentMobile/PermagentMobile/VoiceView.swift`, `VoiceProtocolTypes.swift`, `APIClient.swift`, `ModelPickerView.swift`, `Shared/WatchBridge.swift`, `HubWatchRelay.swift` | Source and generated Xcode target exist; protocol-level evidence is separated below from device evidence. |
| watchOS | `ios/PermagentMobile/PermagentWatch/WatchRelay.swift`, `WatchRecorder.swift`, `WatchHomeView.swift`, shared `WatchBridge.swift` | WatchConnectivity is the documented watch→iPhone hop. Queue/transfer/watchdog source exists; no live watch journey was run. |

Unconfirmed ownership that must remain open for later C1/C2 inventory:

- exact route-to-Command-Center component bindings for every status projection;
- current active worker/card ownership within the dirty worktree;
- actual local model inventory, probe health, and billing classification on the
  target machine;
- live iOS pairing/daemon endpoint and physical watch reachability;
- whether any currently modified platform file belongs to this companion or a
  parallel active program.

## Verified package names and concrete test commands

`cargo metadata --no-deps --format-version 1` confirmed these workspace package
names: `permagent` (`crates/goose`), `permagent-daemon`
(`crates/goose-server`), `permagent-cli` (`crates/goose-cli`), and
`permagent-eval` (`crates/permagent-eval`). Therefore the daemon command uses
`-p permagent-daemon`; old `goose-server` package references are not current.

| Scope | Concrete command | C0 evidence/status |
|---|---|---|
| Core targeted Rust | `cargo test -p permagent --lib <filter>` | Correct package/shape from source and existing DAG instructions; not run by C0 to avoid broad/duplicate execution. |
| Daemon targeted Rust | `cargo test -p permagent-daemon --lib routes::voice::tests -- --nocapture` | Existing V2 receipt says the binary compiled then was SIGKILLed before test execution (~655 MiB); not a pass. |
| Daemon integration | `cargo test -p permagent-daemon --test trust_boundary --test stream_lifecycle --test liveness_wire` | Test targets exist; not run by C0. C1 owns new fault fixtures. |
| CLI/library source check | `CARGO_INCREMENTAL=0 cargo check -p permagent-cli --lib` | Passed in B5.3 receipt; compile only. |
| Daemon/library source check | `CARGO_INCREMENTAL=0 cargo check -p permagent-daemon --lib --message-format=short` | Passed in B5.3/V2/V6 receipts; compile/typecheck only, not fresh executable. |
| Evaluation package | `cargo test -p permagent-eval --lib` | Package verified by metadata; no current C0 result. |
| Voice report fixture | `python3 scripts/test_voice_latency_report.py` | **Executed in C0; exit 0.** Deterministic privacy-safe report fixture only. |
| Command Center | `pnpm --dir ui/command-center test` (script: `vitest run`); targeted form `pnpm --dir ui/command-center exec vitest run src/lib/costMeter.test.ts` | Package script/path verified from `ui/command-center/package.json`; not run in C0. |
| iOS/Watch source/test target | `xcodebuild -project ios/PermagentMobile/PermagentMobile.xcodeproj -scheme PermagentTests ...` and `xcodebuild -scheme Permagent ...` | Target names confirmed in generated project. Existing V6 receipt records exact bounded build/install outcomes; no unchanged rerun. |

`git diff --check` also exited 0 for the current worktree. It is a whitespace
check, not a compile or runtime result.

## Baseline evidence matrix

| Surface | Source evidence | Compile/link evidence | Runtime/test evidence | Device/installed evidence | Current C0 classification |
|---|---|---|---|---|---|
| Core/daemon accounting | Budget/ledger/reservation/worker paths present; B5/B6 audits map seams | B5.3 daemon and CLI `cargo check` passed | Focused B5.3 runtime not promoted; B6 integrated faults partial | No fresh daemon executable from this HEAD; prior bounded build hit disk exhaustion | **Not qualified** |
| Voice backend | `routes/voice.rs` has typed capture/turn/segment/terminal paths | V2/V6 daemon `cargo check` passed | Python report fixture passed; Rust voice test binary SIGKILL before assertions | No source-correlated fresh daemon voice turn | **Deterministic fixture only** |
| iOS voice/chat | Swift sources, generated project, Voice/ModelPicker/WatchBridge present | V4 `VoiceIdleTests` target build linked; V6 app build later passed after changed target selection | V4 simulator protocol tests report `VoiceIdleTests` 15/15; V3 reports prior 5/5 focused tests, with a later identical simulator attempt infrastructure-failed | V6 install/launch and one unpaired screenshot passed; no paired voice/mic/audio/device journey | **Source/protocol evidence; device gate open** |
| watchOS relay | WatchRelay queue, transfer IDs, watchdog, recorder, and shared contract present | Included in generated Xcode project; no standalone C0 compile result | `WatchBridgeTests.swift` exists; no current run result | No watch simulator/physical watch evidence | **Unrated** |
| macOS/Command Center | React/Tauri voice, build, cost, browser, and run/status components present | No current C0 TypeScript build/test result | No current C0 UI test result | No installed desktop journey result | **Unrated** |
| Worker/computer permissions | Rust platform-extension and macOS automation source boundaries present | No current C0 focused compile/test result beyond package checks above | No integrated approve→dispatch→deny/cancel/restart journey | No app/computer tool run | **Unrated** |
| Spectral recognition/correction | Recognition sink, provenance fields, session integration documented | No current C0 compile/test result for recognition changes | No task-scoped recall/correction/revocation measurement run | No cross-device recall journey | **Unrated** |

Historical retained measurements exist in the coding-harness and voice docs,
but C0 does not treat them as a current baseline when source/build identity or
runtime execution is missing. In particular, a retained local coordinator smoke
is documented as 0/1 and red, while V2/V6 voice records distinguish source
compile from SIGKILL/CoreSimulator failures. No fresh cost-per-verified-success
or local-vs-cloud paired measurement is present.

## Platform journey matrix

| Journey required by C0/C7 | Existing source/test anchor | Evidence level present | Missing gate |
|---|---|---|---|
| macOS/Command Center starts a bounded coding turn and shows truthful run/cost/status | `ui/command-center/src/components/build/*`, `lib/costMeter.ts`, `routes/coding_session.rs`, `routes/runs.rs` | Source only; B5.3 daemon/CLI compile evidence | Current UI test run, live daemon projection, reconnect/history hydration, installed desktop journey |
| iOS captures voice, receives terminal outcome, shows transcript/reply/model state | `VoiceView.swift`, `VoiceProtocolTypes.swift`, V2/V3/V4 receipts, `routes/voice.rs` | Source + protocol compile; V4 pure/simulator protocol evidence | Fresh paired daemon/app, microphone/audio playback, reconnect/interruption, accessibility, live model switch |
| Watch queues offline recording and relays through iPhone without duplicate/lost completion | `WatchRelay.swift`, `WatchBridge.swift`, `HubWatchRelay.swift`, `WatchBridgeTests.swift` | Source contract only | Watch test execution, offline/online transfer trace, duplicate request/response correlation, physical watch |
| Model/device change applies next turn and preserves active turn | `ModelPickerView.swift`, `routes/config_management.rs`, V5 receipt | Source/read-only contract evidence | Fresh endpoint interaction across macOS/iOS/watchOS and matching build IDs |
| Recognition/correction/recall is attributed and privacy-scoped | `recognition.rs`, `recognition_sink.rs`, Spectral integration docs | Source boundary only | Fresh no-provider fixture with correction/revocation/restart and cross-operator isolation |
| Computer tool work is approved, scoped, cancellable, and recoverable | `dispatch_scope.rs`, `write_scope.rs`, `app_conductor.rs`, `browser.rs`, `computercontroller/platform/*` | Source boundary only | No-model approve→dispatch→deny/cancel/restart integration journey |

## Missing baseline evidence and handoff

The following remain explicit C0 debt:

1. clean or attributed source baseline for the companion work;
2. observable active goal/card and owner identity;
3. fresh daemon executable built from `737c969...` (the prior V6 build failed
   with `No space left on device`, and the old binary was not relabelled);
4. focused daemon runtime tests that execute assertions rather than SIGKILL;
5. current Command Center typecheck/test and live status/cost projection;
6. paired iOS voice journey on a fresh daemon/app, including microphone,
   playback, reconnect, model selection, and accessibility;
7. WatchConnectivity offline queue/reconnect/duplicate-correlation evidence;
8. measured local/cheap/cloud capability, latency, authoritative cost, and
   verified-success dataset under the same task IDs and limits;
9. recognition correction/revocation provenance and cross-device isolation
   evidence; and
10. permission-bounded computer-tool approval/cancellation/restart evidence.

## Receipt outcome

`C0` is **not promoted**. C1 may use this receipt for ownership and evidence
triage, but the companion master must remain at `c0_evidence_and_ownership:
active` until the missing baseline decisions are resolved. No C0 status change,
production edit, provider call, or duplicate B5/B6 test was made.
