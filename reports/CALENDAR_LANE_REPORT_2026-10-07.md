# Calendar lane report — 2026-10-07

Worktree `/Users/j/Documents/dev/permagent-worktrees/wq-calendar`, branch `wq/calendar`, base `bcfa4e1f`, resumed from checkpoint `a32d78e2`.
Worker: Claude (claude-opus-5-5), one worker and no subagents, under Codex coordination.
**Nothing is committed in this session** (as instructed). The checkpoint commits `93b6d09f`, `78043294` and `a32d78e2` came from the earlier worker. Their `Co-Authored-By: Claude Fable 5.1` trailer names that worker's model, not this one. Any commit of this session's edits should name claude-opus-5-5.

## What this session did

I reviewed the whole diff `bcfa4e1f..a32d78e2` against the brief, fixed two defects and added a test for each:

1. **Shortcuts leaked out of the event popover.** The popover is portalled to `<body>`, but React still bubbles its synthetic key events to the card's `onKeyDown`. Pressing ←/→, T, D, W or M inside the popover paged or switched the calendar behind it. Fix: the card now ignores keys whose DOM target is outside the card root (`CalendarCard.tsx`, `onKeyDown`). Test: `popover > keys pressed inside the popover do not page the calendar behind it`.
2. **A failed read was shown as an empty calendar.** `useCalendarEvents` reported `failed`, but the card never used it. When access was granted but EventKit and the daemon both failed, the card said "No events this week". Now it says "The calendar could not be read just now" and offers a **Retry** button (`data-testid="calendar-failed"` / `calendar-retry`). This only happens with `fullAccess` in the desktop app; in the other cases the access panel already explains the gap. Test: `read failure > says the calendar could not be read (not "No events") and Retry asks again`.
3. Test hygiene: `CalendarCard.test.tsx` now resets the `invoke` mock to a named `serve` implementation in `beforeEach`, so a test that overrides it cannot affect the tests after it.

## Files (whole lane, vs bcfa4e1f)

Frontend (`ui/command-center/`):
- `src/components/dashboard/cards/CalendarCard.tsx` — the card: header with period title, Today, ‹ ›, a Day · Week · Month tablist, the Next-up strip, the compact mode and the popover. **Edited this session.**
- `src/components/dashboard/cards/CalendarCard.test.tsx` — 16 tests. **Edited this session** (+2 tests, mock reset).
- `src/components/dashboard/cards/calendar/calendarModel.ts` + `.test.ts` — ranges, the 42-cell month grid, chips with +N, the overlap layout, next-up and countdown, and calendar colour from theme hues (no raw hex).
- `src/components/dashboard/cards/calendar/useCalendarEvents.ts` + `.test.tsx` — the range hook. Source order: EventKit, then the daemon, then none. Cache is per source and range; it refreshes on focus and every 5 min, and never blocks render.
- `src/components/dashboard/cards/calendar/CalendarViews.tsx` — the Month, Week and Day drawings.
- `src/components/dashboard/cards/calendar/EventPopover.tsx` — the detail popover, with Open link (http(s) only) and Open in Calendar.
- `src/components/dashboard/cards/useShellCalendar.ts` — new `loadToday` flag. The card keeps access state and actions but skips the old read of today's events.
- `src/components/dashboard/cards/registry.ts` — `calendar` is now a first-party code card, and `CALENDAR_SIZE_PRESETS` is added (compact 4×4, medium 7×8, large 12×9).
- `src/components/dashboard/cards/calendarPermission.test.ts` — pins the new shell commands and the `EKParticipant` feature.
- `src/components/dashboard/Dashboard.tsx` — `calendar` grows to fit its content.
- `src/components/dashboard/templates.ts`, `useLayout.ts` — the calendar defaults to medium (7×8) in the templates, the default layout and `ensureCalendarCard`.
- `harness/calendar/*`, `vite.calendar-harness.config.ts` — a throwaway evidence harness: 40 mocked events and a pinned clock (Wed 7 Oct 2026 10:18). **Kept so the parent can use it for screenshots. Delete it before the final merge commit**, as the brief requires.

Native (`ui/desktop/src-tauri/`) — **UNCOMPILED** (no cargo in this lane):
- `src/calendar.rs` — new `calendar_events_in_range(start, end)`, with RFC 3339 bounds, a cap of 62 days and `spawn_blocking`. Also new: `open_calendar_app` (a fixed `/usr/bin/open -a Calendar`), and `id`, `attendees` and `url` fields on `CalendarEvent`. `calendar_upcoming_events` is unchanged in behaviour and shares `events_between`. Unit test `parse_range_accepts_a_month_and_rejects_bad_windows` is added.
- `src/main.rs` — the two new commands are registered.
- `Cargo.toml` — `EKParticipant` feature on `objc2-event-kit`.

Reports: `reports/calendar-evidence/*.png` (18 screenshots from the earlier worker; see the screenshots section) and this report.

## Checks (run from `ui/command-center`, this session, after the fixes)

| Check | Result | Exit |
|---|---|---|
| `npx vitest run` on the calendar files (`cards/calendar`, `CalendarCard*`, `calendarPermission`, `calendarAccess`) | 6 files, 79 tests passed | 0 |
| `npx vitest run src/styles src/components/common src/components/dashboard` (convention scanners spaceScale, rawButton, backdropFilter, textScale, radiusScale, etc., plus every dashboard test) | 82 files, 691 tests passed | 0 |
| `npx tsc --noEmit` | clean | 0 |
| `npx vite build --logLevel error` | built; the only output was a Browserslist "data is 6 months old" notice | 0 |
| Grep of the lane's frontend files for the developer's first name and legacy agent names | none | — |

Exit codes were taken from `<cmd> && echo …_EXIT_0`, i.e. the command's own status, not a pipe's.

**Not run, per instructions:** the full `npx vitest run` suite (the brief wants it once at the end, but this session was told not to run it). Also not run: cargo/`cargo check`/`cargo test`, native or Tauri builds, install, daemon restarts, UI automation and Playwright screenshots. `git status` was refused by the sandbox from this session, so I could not confirm the working tree directly. The intended working-tree changes are `CalendarCard.tsx`, `CalendarCard.test.tsx` and this report. `vite build` writes `dist/`, which I expect is gitignored, but I have not confirmed that.

## Rust/Tauri status — uncompiled; risks for the coordinator's cargo gate

I could not read the cargo registry from this sandbox, so these points are reasoned, not compiled:
- `event.URL()` → `NSURL::absoluteString()` needs the `NSURL` feature of `objc2-foundation`. Our `objc2-foundation` line lists `NSString, NSValue, NSDate, NSError, NSArray`, not `NSURL`. It compiles only if `objc2-event-kit` turns `NSURL` on itself (with `default-features = false` it may not). **If the gate fails here, add `"NSURL"` to `objc2-foundation`'s features.**
- `event.attendees()` returns `NSArray<EKParticipant>`. It is gated on the newly added `EKParticipant` feature, and `.count()` needs `NSArray` (present).
- `event.eventIdentifier()` is on `EKEvent` (present).
- `chrono` is already a dependency, so `parse_range` should compile on every target (`#[cfg(any(target_os = "macos", test))]`).
- Suggested gate: `cargo check -p permagent-app` plus `cargo test -p permagent-app --lib -- parse_range day_window`, through `scripts/build-guard.sh` with the shared target dir. Only the coordinator may run these.

## Brief coverage and what is still open

Done in code and tests: the range hook with fallback order and cache; Month (42 cells, 3 chips, "+N" opens the day); Week (7 columns, all-day row, now-line, overlap layout); Day (timeline with the list beside it from 560 px wide); the popover; keyboard ←/→ T D/W/M plus Esc; the view saved in localStorage; Next up with a minute countdown; the compact mode below 420 px card width; theme-hued calendar colours; empty states; the access panel; and reduced motion (`runViewTransition` respects `getReduceMotion`).

Open or partial:
- **Size presets are declared but nothing reads them.** No Layouts or Customize surface has a per-card size picker; sizes change with the corner grip. Compact is reached by dragging the card below ~420 px (about 4 columns at 1244 wide). Wiring a picker would add a new UI concept, so I left it out. Coordinator's call.
- **The view is saved per app, not per layout.** The brief asked for per-layout saving "if a pattern exists"; there is no card-settings pattern, so it uses the card's own key `permagent.dashboard.calendar.view.v1`.
- **Calendar colour comes from the calendar's name, not EventKit's colour.** This is deliberate (theme-safe, works in both themes); the bridge does not send the colour.
- **Old saved layouts keep their size.** A layout with the old calendar size (5×4) keeps it. Because the card grows to fit, it renders the full calendar at 5 columns (~460 px at 1440), which is tight in Week view. A migration was not added.
- **The daemon fallback only knows today.** Week and Month views say so in the empty-state hint.
- **"Open in Calendar" opens Calendar.app, not the specific event.** EventKit has no public deep link to one event.

## UI verification route (for the parent — screenshots are not accepted by me)

1. Harness (no daemon, no native app): from `ui/command-center` run `npx vite --config vite.calendar-harness.config.ts`. It serves on port 5391. Open `http://localhost:5391/?view=week&cols=7&theme=dark`. URL parameters: `view` = day|week|month, `cols` = 4 for compact or 7/12, `theme` = light|dark. Check at 1244×879 and 1440×900.
2. Points to look at: the Month "+N" chip and day click; Week overlaps in the Wed 14 Oct cluster; the now-line on today; the popover's position near the window edges; the failure/Retry state (needs `invoke` to reject — not in the harness).
3. Real app (only after the cargo gate and a native build): Home → Calendar card. Grant access via the access panel, then page through Week and Month to confirm `calendar_events_in_range` returns real events. Compare attendees and the URL against Calendar.app.
4. `reports/calendar-evidence/*.png` came from the earlier worker, before this session's fixes. The two fixes do not change the default rendering, but those PNGs are not acceptance. Retake them.
