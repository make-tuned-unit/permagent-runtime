# Functional wiring audit — October 18, 2026

This audit follows the installed dogfood release `55d5ae293` (source receipt `71d7f73d`). It checks navigation, input and submission contracts, persistence, error/retry behavior and native handoffs. A passing mocked handler test is recorded separately from a real HTTP persistence check and a native UI check. External actions are not represented as verified just because their button renders.

## Verification work

| Gate | Work | Status |
|---|---|---|
| D24 | Page/control/flow inventory and evidence boundaries | Verified, scoped reports below |
| D25 | Interaction regressions and concrete wiring fixes across all primary surfaces | Verified |
| D26 | Full daemon regression and isolated real HTTP/browser persistence | Verified |
| D27 | Installed app navigation and native surface checks | In progress |
| D28 | Integrated UI/type checks, review, and rebuilt dogfood update if product changes | In progress |

Live checks use normal navigation. Record edits and persistence checks use synthetic isolated records; no messages, model jobs, financial transactions, recordings or production automation runs are used as test inputs. External/provider-dependent flows will retain explicit verification limits.


## Findings and changes

1. Notification tab/URL activation acknowledged unread items even when no workspace hosted the destination. The shared destination helper now reports success; failed destinations remain unread, leave the tray open and do not dismiss the toast on that click. Browser URL requests are only queued after Build is reachable. Profile links now explain an unavailable Build destination.
2. Finance's “View in World” changed workspace without focusing the Financier promised by its tooltip. It now uses the existing agent-focus route.
3. World stage readiness waited for optional architecture and planting in addition to terrain/collision. Initial native entry showed a timeout notice over a rendered scene; Retry recovered. Stage readiness now follows the core terrain/collision build, with progressive detail builds retaining their own asset health. A mounted HabitatScene regression would fail with the prior gate.
4. History lacked explicit tab-to-panel accessibility relationships. Named focusable panels and bidirectional ARIA references were added. Native AX once omitted Spend's content even though a screenshot showed it rendered; a second navigation exposed the tab group. The intermittent AX cause is unproven, and the semantics change is not a claimed root-cause fix.

## Executed evidence

- **Final integrated UI suite: 568 files / 4,657 tests passed**, 144.20s: `../ux-dag/flow-audit-full-ui-final-20261018.log` (generated local log). TypeScript passed: `../ux-dag/flow-audit-typecheck-20261018.log`.
- An interim parallel run had three failures: two profile-link fixtures lacked a Build workspace under the corrected contract; their success fixtures now include one and failure tests assert feedback/no pending URL. The Home reset test now awaits and scopes its confirmation dialog. The complete final suite above includes those corrections.
- **Daemon suite: 1,208 passed, 0 failed, 5 ignored**, 173.68s. Guard log/receipt `/Volumes/PermagentBuilds/permagent/logs/20261018-184911-21457.log` / `.json`, exit0. Reused exact test artifact `debug/deps/permagent_daemon-1ea2f0987f1cfb61`; Rust/Cargo trees are unchanged from its recorded `8be925892f24bfd158a80722dc9292158c120d8a` source. Test runner recorded current source `71d7f73d7b785dfa527a387c94292e8dad4174eb` and checked clean Rust sources. This was execution of a verified existing artifact, not a new backend compilation.
- **Explicit isolated HTTP/browser gate: 1 passed**, 272.56s. Guard log/receipt `/Volumes/PermagentBuilds/permagent/logs/20261018-185452-27772.log` / `.json`, exit0. Safari used current production components and only the restricted port3002 proxy. Project Paused→Active persisted after reload. Overview opened the exact synthetic task; Details detected React/Node.js from its manifest. Person Notes saved, closed and reopened with the same value. Automate listed paused schedules/empty activity; History returned its empty session state. The proxy deliberately rejects calendar import, so the displayed calendar error is a harness boundary.
- Six independent **read-only** HTTP checks then verified the browser's status and notes mutations, detected stack, task and schedule/session envelopes, before authorizing test completion. The daemon independently checked persistence and that discovery created no manual stack records. No extra PATCH from the independent client could mask a failed UI save. See [browser-persistence-receipt.json](browser-persistence-receipt.json). The runner removed private credentials and stopped its owned daemon; the owned browser tab/server were closed.

## Installed-app walkthrough before fixes

The installed `55d5ae293` app was tested via native UI: Build and its project picker; Automate details, Activity, run detail and Open conversation; Brain populated Graph→List; World loading and Retry; Record modal validation and Cancel; all13 Settings panes; Finance Household/Holdings/Research and its World link; History Sessions/Downloads/Activity/Spend; return to Projects. Existing automation failure notices were displayed but no automation was run or reconfigured. These are concrete navigation/render checks, not proof that every downstream external action succeeded.

## Coverage and boundaries

Detailed matrices: [Home, Projects, People, notifications](home-projects-people.md), [Settings, Automate, Finance, Brain, World](settings-automate-finance-brain-world.md), and [Grow, Build, History, Record, chat](grow-build-history-record.md). They distinguish exercised controls from source tracing and mocked transport. The integrated suite supersedes interim test failures/typecheck reports in the agent notes.

No assertion of “all buttons work perfectly” is made. Model/provider requests, paid services, real Cursor/model execution, outbound messages or publication, pairing/credential grants, microphone/system-audio capture and financial execution were not performed against live accounts. Their mock/native-command contracts and guards are covered where cited; successful live external delivery remains unverified. Appearance/Shortcuts were navigated natively but lack new dedicated control-interaction coverage in this pass. The other four ignored daemon tests need external model/voice fixtures; the fifth was the staging gate run separately above. Normal view mount effects may run; no live records were manually edited or test work submitted.

D27/D28 remain open for post-build native checks and installation of these fixes.
