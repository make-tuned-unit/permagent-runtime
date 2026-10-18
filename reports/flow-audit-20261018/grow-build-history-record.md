# Grow, Build, History, Record, and chat flow audit

Audit date: 2026-10-18  
Checkout: /Users/j/Documents/dev/permagent-worktrees/world-production-20261006

## Outcome

No broken UI-to-store wiring was found in the owned surfaces during this pass. Existing tests already cover most high-risk outcomes with mocked transport/native seams. I added outcome regressions for History panel switching and chat submission. Native dogfood initially reported missing History content in the macOS accessibility tree, though a repeat navigation exposed the tab group and selected Spend panel. The DOM had no aria-hidden/inert condition; the tabs lacked explicit tab-to-panel relationships. I added named, focusable tabpanel wrappers plus aria-controls/aria-labelledby relationships and assertions. The AX report appears intermittent and its root cause remains unproven; native post-build verification is still needed.

## Flow coverage

| Surface | User action and outcome traced | Existing regression evidence |
| --- | --- | --- |
| Grow actions | Select category tabs; review a durable action; send an action directive to the selected coding agent; persist the launch in pendingTerminalLaunch with project root and action metadata | GrowActions.board.test.tsx checks category filtering, deliberate review, exact Cursor command quoting, growthAction, rootPath, and no duplicate follow-up paste. codingAgents.test.ts checks safe command construction against a fake executable. |
| Grow results | Request measurement and handle accepted/refused/error results | GrowResults.test.tsx, growthResults.test.ts |
| Grow strategy | Switch strategy/analytics/results/calendar/action lenses and render strategy controls/data | growLenses.test.ts, growStrategy.test.ts, GrowView.header.test.tsx |
| Grow calendar | Load and refresh entries; approve, retry, reschedule, delete, generate media, and surface failures | GrowView.calendar.test.tsx; API calls are mocked |
| Grow analytics | Load actual metric shapes through mocked API, render chart, stats, funnel/coverage, empty/error states | GrowAnalytics.test.tsx, DailyPageviewsChart.test.tsx, FunnelPanel.test.tsx, analyticsFormat.test.ts |
| Build launch | Consume a queued terminal launch once, unhide terminal when needed, and pass command, working directory and metadata to terminal-tab creation | BuildView.pendingLaunch.test.tsx mounts the real BuildView/store/claim function and mocks the terminal manager. Grow's launch test supplies the producer-side contract. |
| Build panes and project actions | Toggle terminal/browser visibility, cycle panes, open project/site actions and route a pending launch | BuildView.rail.test.tsx, paneTabCycling.test.ts, ProjectChip.test.tsx, pendingLaunch.test.tsx |
| Terminal | Spawn/reattach, deliver initial commands/follow-up input, persist and restore tab state, pop out, and process stream/lifecycle events | Terminal.lifecycle.test.ts, Terminal.browser.test.tsx, Terminal.followUp.test.tsx, Terminal.reattach.test.tsx, TerminalManager.persistence.test.tsx, TerminalManager.realmReload.test.tsx, TerminalManager.freshProfile.test.tsx, followUpDelivery.test.ts, terminalReattach.test.ts |
| History | Select Sessions, Downloads, Activity and Spend, displaying one selected segment and its matching panel | Added panel-switch and bidirectional tab-to-tabpanel relationship regression in HistoryView.tabs.test.tsx; SpendPanel.test.tsx covers cap save/error behavior. Other panels have dedicated tests (SessionsList, InboxPanel, ExecutionTrace). |
| Record | Choose capture source, keep Start disabled without a source, gate on permissions/readiness, stop and focus the saved note | MeetingRecorder.sources.test.tsx, MeetingRecorder.clearance.test.tsx, MeetingRecorder.focus.test.tsx, MeetingRecorder.test.tsx, and useMeetingDictation.lifecycle.test.tsx. Source selection/network calls are mocked. |
| Chat entry and composer | Open dock/window fallback, preserve clearance/size, submit text, set up or stop a session, and handle streaming/auth errors | ChatLauncher.clearance.test.tsx, ChatLauncher.publish.test.tsx, ChatInput.setup.test.tsx, ChatInput.stop.test.tsx, ChatView.lifecycle.test.tsx, ChatView.setup.test.tsx, MessageList.*.test.tsx. Added ChatInput submission outcome in ChatInput.send.test.tsx. |

The Grow-to-Build handoff is verified across producer and consumer contracts: the Grow test asserts the exact shell-quoted cursor-agent launch and stored metadata; an added BuildView test creates the same prompt through the real coding-agent command helper and proves the real consumer passes that exact initial command, root path, growth-action attribution and absent follow-up input to terminal-tab creation; terminal delivery/reattach tests exercise initial command and follow-up metadata. These tests use fake terminal tabs/PTY or mocked transports and do not invoke Cursor, Claude, another harness, or a model.

## Changes and test results

- HistoryView.tsx: each tab points to its active role=tabpanel with aria-controls; the panel has a stable id, aria-labelledby back to its tab, and keyboard focusability. HistoryView.tabs.test.tsx clicks each segment and verifies the matching panel is mounted, all others are absent, exactly one tab is selected, and both relationship directions are valid.
- ChatInput.send.test.tsx: added a mocked-store regression that types padded text, clicks Send, verifies one call with the trimmed prompt, and checks the composer clears.
- BuildView.pendingLaunch.test.tsx: added the Cursor Grow-to-Build consumer contract test for exact inline prompt plus action metadata.
- Focused run: npm test -- --run src/components/history/HistoryView.tabs.test.tsx src/components/chat/ChatInput.send.test.tsx — 2 files, 9 tests passed. After the tabpanel change, npm test -- --run src/components/history/HistoryView.tabs.test.tsx src/components/history/SpendPanel.test.tsx — 2 files, 9 tests passed.
- Wiring run: npm test -- --run src/components/build/BuildView.pendingLaunch.test.tsx src/components/grow/GrowActions.board.test.tsx src/components/terminal/Terminal.followUp.test.tsx — 3 files, 49 tests passed.
- Full npm test -- --run completed: 566 files, 4,649 passed, 3 failed. Dashboard.firstAction.test.tsx failed to find “Reset dashboard?”; PersonDetailModal.links.test.tsx had two failures expecting pendingBrowserUrl to be set. These are outside this change; the latter overlaps the concurrent openInBrowser return-value change and was reported to root for integration.

## Boundaries

Tests exercise mocked HTTP/store results and fake terminal/PTY executables. This pass did not start an actual coding harness, invoke a model, send a real message, capture microphone/system audio, mutate the user's profile/data, or launch the native app. macOS Tauri commands, OS audio permissions, actual PTY process creation, live-daemon/network behavior, and the native accessibility tree remain outside this UI test evidence. Native post-build verification should check the History tabpanel relationship; the original intermittent AX omission has no confirmed root cause.
