# Home, Projects, People, and notification flow audit

Date: 2026-10-18  
Scope: synthetic UI tests and read-only source tracing in `ui/command-center`; no live profile or application actions were performed.

## Findings and fixes

Notification activation previously acknowledged an item even when its destination could not be reached. `NotificationHost` closed the tray and marked the item read without checking the route result. URL notifications also queued `pendingBrowserUrl` before knowing whether a Build workspace existed, allowing an old link to be opened on a later unrelated Build visit. The routing helper now returns success; notification tray, toast, and operating-system notification paths acknowledge only after successful activation. `openInBrowser` now returns `false` and leaves no pending URL when Build is unavailable. Failed toast activation keeps the toast visible. Custom notification actions and notifications with no destination retain their existing successful-activation behavior.

Added regression coverage for target navigation success/failure, Build URL navigation success/failure, and failed toast activation. Added a People calendar import failure/retry test and a Home reset confirmation/persistence test. Profile-link failures in the People detail modal now explain that a Build tab is needed; profile links only queue a URL when that route succeeds.

## Executed verification

From `ui/command-center`, this exact focused command passed:

```sh
npx vitest run src/components/notifications/NotificationHost.navigation.test.tsx src/components/notifications/NotificationHost.test.tsx src/components/notifications/NotificationHost.decisions.test.tsx src/components/notifications/Toast.test.tsx src/components/people/PeopleView.calendarRetry.test.tsx src/components/people/PeopleView.navigate.test.tsx src/components/people/PeopleDirectory.test.tsx src/components/people/PeopleView.test.tsx src/components/projects/ProjectsView.drilldown.test.tsx src/components/projects/ProjectWorkspace.navigation.test.tsx src/components/dashboard/Dashboard.firstAction.test.tsx src/components/dashboard/Dashboard.focus.test.tsx src/components/dashboard/LayoutsGallery.test.tsx src/components/dashboard/useLayout.ready.test.tsx
```

Result: **14 test files passed; 122 tests passed** (Vitest, 2.41 seconds). TypeScript check also passed:

```sh
npm run typecheck
```

Result: `tsc --noEmit`, exit code 0.

These tests use mocked API/store data and DOM events. They did not contact the live app or perform live writes.

During integration review, the pre-existing PersonDetailModal profile-link tests were updated to model a Build workspace for the success cases. Their original store fixture had no Build destination, so `openInBrowser` correctly declined to queue a URL. An explicit unavailable-Build case now verifies that failure behavior. The combined follow-up run passed:

```sh
npx vitest run src/components/dashboard/Dashboard.firstAction.test.tsx src/components/projects/PersonDetailModal.links.test.tsx
```

Result: **2 test files passed; 32 tests passed**. The Home reset-confirmation test also passed in this combined run.

The same combined run was repeated after tightening the reset test to scope its confirmation check to the owned dialog and after adding People profile-link failure feedback; it again passed **2 test files / 32 tests**. `npm run typecheck` also exited 0 after these updates.

## Flow coverage

The classifications below distinguish tests executed during this audit from code-path inspection. “Executed” means a synthetic test exercised the UI/API seam; it does not imply live server acceptance.

| Area | Flows audited | Evidence |
|---|---|---|
| Home | First-action/empty and returning dashboard rendering; responsive/theme/dock combinations; daily/custom layout handling; customize and reset menu; reset confirmation and persistence; layout gallery preview/apply/keep; focus and collapse behavior. | **Executed:** `Dashboard.firstAction.test.tsx`, `Dashboard.focus.test.tsx`, `LayoutsGallery.test.tsx`, `useLayout.ready.test.tsx`. The added reset test verifies no write before confirmation and persistence of `DEFAULT_LAYOUT` after confirming. |
| Projects | Project list loading/error/retry; search and shelved/status filtering; duplicate-status guard; project selection and workspace drilldown; stale navigation recovery; board navigation and card/status interactions. | **Executed:** `ProjectsView.drilldown.test.tsx`, `ProjectWorkspace.navigation.test.tsx` plus existing focused tests in the suite. **Static:** route implementations and API call paths were traced; no live project mutation was made. |
| People | Graph/list navigation; directory search and cohort behavior; person creation request body; duplicate handling; pending-person navigation; calendar import failure and retry. | **Executed:** `PeopleView.test.tsx`, `PeopleView.navigate.test.tsx`, `PeopleDirectory.test.tsx`, and the added `PeopleView.calendarRetry.test.tsx`. The retry test verifies two POST attempts to `/api/people/calendar/import`, success feedback, and `peopleRev` increment. **Static:** endpoint implementation was traced in `crates/goose-server/src/routes/people.rs` (calendar import route). |
| Notifications and navigation | Tray target routing; linked URL routing to Build; failure when target/Build is missing; read acknowledgment and tray close on success; preservation of unread/open state and URL queue on failure; toast failure remains visible; custom activation, mark-all-read, decision and toast flows. | **Executed:** `NotificationHost.navigation.test.tsx`, `NotificationHost.test.tsx`, `NotificationHost.decisions.test.tsx`, `Toast.test.tsx`. **Static:** operating-system notification click uses the same shared activation helper and only marks read on success; no native OS notification was triggered. |
| Projects server endpoints | Project list/status and board/card route call shapes and route definitions. | **Static only:** source inspection in the server route modules (including `crates/goose-server/src/routes/cards.rs`). UI tests use mocked API boundaries; no API server mutation or end-to-end request was issued. |

## Remaining limits

- No live browser session, daemon/API integration run, persistence reload, or real notification delivery was exercised. Parent/root is handling isolated browser/API acceptance.
- Operating-system notification delivery/permission behavior is static-only; its click handler delegates to the tested shared helper.
- Project write routes (status/card create/update/delete/cancel/due-date) were traced and existing focused component tests were included where present, but this audit did not send requests to a live server.
- The focused Vitest process emitted environment warnings (existing websocket-port, browser-pairing, React `act`, and caniuse-lite notices); the command exited successfully with all listed tests passing.
