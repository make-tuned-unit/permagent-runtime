# First-user Home, Calendar and World acceptance

Source checkpoint: `7f4046c3` plus the focused changes described below. This is a source-audit and test receipt, **not a runtime acceptance pass**. Read alongside `FLOW_MATRIX_2026-10-07.md` and `SESSION_CLOSEOUT_2026-10-08.md`; historical screenshots or navigation alone do not close a flow.

## Test environments and receipts

Use a disposable, genuinely empty app root for first-run checks. The existing populated review root cannot establish first-user behavior. Distinguish (a) an unpaired browser, (b) a paired daemon with no projects, sessions, calendar, or memories, and (c) a native app with Calendar permission not yet granted. Use only owned fixtures for writes. Do not grant access to personal calendars, folders, or microphones merely to make an empty state disappear.

For each CUA row, record frontend/source identity, backend/root, viewport, actual actions, observed outcome and screenshot. Refind controls after every navigation, modal, scroll, or responsive change. Pending means not checked in this pass; unit tests do not change that status.

## Priority CUA checklist

All rows below are pending current runtime verification.

| Priority | Path and actions | Expected visible result |
| --- | --- | --- |
| P0 | Fresh root → onboarding; choose Decide later/no model, no key, skip optional folders/extras; Back then Next through identity/review. | Values survive Back; pending/unavailable capabilities are explained; skipped setup is not represented as configured. No permission grant or model request occurs from viewing a step. |
| P0 | Fresh root → finish setup once; reload/reopen. | One completed setup transition; Home opens on restart without looping onboarding. No automatic first message or fabricated agent activity. Any failure exposes recovery without losing entered values. |
| P0 | Empty paired Home → inspect every card from top to bottom. Repeat in unpaired browser. | Empty counts and guidance reflect absent data; unread/offline data is not reported as a successful empty read. No indefinite spinner, placeholder person/project, overflow, or unusable primary action. Unpaired state has a useful route to connection. |
| P0 | Home Calendar, no source → read compact header/list → expand → Day, Week, Month. | Header says Calendar not connected; list/Day invite connection rather than claiming No events. Native/browser instructions match actual capability. Merely opening the card does not request Calendar access. |
| P0 | Native Calendar with an owned permission fixture: granted but read fails → compact Retry → recover. | Calendar unavailable / failed-read message is visible with Retry. A successful retry replaces the failure with current events; no false disconnected or verified-empty state. If no safe fixture is available, leave this row unverified. |
| P0 | World on empty root → Agent desk → each role → Open shared chat. | Name/role/status are truthful; empty conversation explains how to begin. Button opens existing shared conversation without creating or sending a message. Specialist text says only briefs include role context. Offline status has an intelligible explanation. |
| P0 | World → Write a brief → each brief type, including empty fields → review/export. | Empty requirements are visible; editable fields retain input across intended navigation. Preview/export are distinguishable from dispatch. No send occurs from selecting a role, opening a question, or drafting. |
| P1 | Home Calendar with a verified empty native source, then a source with known events. | Verified empty day may say No events today. Events show correct date/time/all-day treatment; upcoming count and next-up agree with visible source. |
| P1 | Calendar using daemon today-only fallback → future Day/Week/Month. | Persistent partial-data notice; an unread future day says it has not been checked. Absence of future events is not represented as a verified empty calendar. |
| P1 | Focus Calendar view tabs with keyboard: Left/Right, Home/End, Tab; use date arrows outside tablist, then D/W/M/T shortcuts. | Tab focus and selected view move together with wrapping; only active view tab is in Tab order. View arrows do not advance the date. Date shortcuts elsewhere continue to work. Reload preserves chosen Day/Week/Month view. |
| P1 | Calendar event → popover → Escape; Home card expand → Escape/collapse. | Popover closes before enclosing card; focus returns to triggering event/card control. Inputs and open dialogs do not accidentally trigger global date/card shortcuts. |
| P1 | Home → Customize/add picker/layout preview → cancel; reopen and compare. | Preview/cancel does not save a new layout. All card controls remain reachable; a deliberate fixture-only save persists after reload. |
| P1 | Home at desktop and 776px/390px viewport; scroll all cards, focus one then collapse. | No clipped banner actions or horizontal content overflow; focus mode preserves compact neighboring cards and returns keyboard focus on close. Small widths are a visual check, not established by source review. |
| P1 | World → settings → Day, Night, System; navigate away/back and reload each selection. | Selection persists locally; scene visibly changes as appropriate. Invalid/missing preference falls back to System. Changing World lighting does not silently change the app theme. |
| P1 | World open portrait/HUD → toggle app reduced motion; test app override against OS preference, then return to System. | Already-mounted portrait responds without remount/navigation; decorative motion and HUD behavior respect the effective preference. System follows OS while explicit app preference takes precedence. |
| P1 | World desk open → close with keyboard; repeat narrow viewport, role selector and details expansion. | Focus returns to the desk trigger; portrait/name/actions fit. Narrow open desk has a short scene above it and reachable actions; closed desk restores immersive scene. No duplicate heading or blank optional identity block. |
| P1 | World Learn → all four branches; Evidence → Arrival choices, Archive choices, Water Shade/Flow predictions; inspect/reset. | Each choice changes the intended content; evidence identifies the actual source/context. Back/reset restores the intended stage without duplicate actions or stale selection. Empty/unavailable evidence is explicit. |
| P1 | World → all six Places → walk complete routes and test boundaries/collisions → Escape/overview; repeat Night. | Every destination is reachable; camera does not enter geometry or fall off navigable areas. Return/overview works from each place. Night remains readable; scene loading/failure never masquerades as a ready empty world. |
| P1 | World → Sit together → close before any explicit audio action. | Opens the existing experience without starting microphone capture merely from viewing it. Any recording check remains separate from World navigation; the known silent native-capture blocker is not closed by this flow. |

## Confirmed source defects fixed in this pass

1. Calendar compact list and focused Day could claim an empty calendar when no calendar had been read. They now distinguish unconnected, verified native empty, and partial daemon reporting. A non-today Day no longer uses “today” in its empty description.
2. Compact Calendar used the full-view range to determine failure, could mislabel a granted-but-failed read as disconnected, and offered no compact retry. Failure now follows the visible range; compact Retry refreshes that range.
3. Calendar's Day/Week/Month controls declared tabs but all remained in Tab order; Left/Right paged dates instead of moving tabs. They now use roving tab focus with Left/Right/Home/End, preserving existing date shortcuts outside the tablist.
4. Specialist World profiles claimed Chat attached role context, although Chat opens the shared conversation unchanged. Copy now distinguishes shared Chat from role-context briefs.

## Validation and limits

Executed in `ui/command-center`:

```text
npm test -- src/components/dashboard/cards/CalendarCard.home.test.tsx src/components/dashboard/cards/CalendarCard.test.tsx src/components/dashboard/cards/calendar/useCalendarEvents.test.tsx src/components/world/forum/ForumView.profile.test.tsx
4 files passed; 42 tests passed (9 + 19 + 8 + 6).
npm run typecheck
Exit 0.
git diff --check
Exit 0.
```

Tests cover no implicit permission request, unread versus verified-empty state, compact failure/retry recovery, view-tab focus/date preservation, existing calendar range/view behavior, and truthful World role-context copy. These are mocked component/source checks. No new browser/native acceptance, model invocation, asset generation, full build, or real Calendar permission/capture check was performed by this worker.

Source audit also found existing dashboard error/retry and cached-data status, Calendar popover focus return, Home focus/Escape guards, and World preference hooks. Their existence is not evidence that the corresponding current runtime flows passed. The narrow Home banner's fixed action group remains a visual item to inspect, not a confirmed source defect.
