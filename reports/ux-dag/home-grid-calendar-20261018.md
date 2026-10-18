# Home geometry and Calendar refinement — October 18, 2026

## User-reported problems

Done shrank cards after Customize, leaving uneven rows and gaps. Calendar's centered date tile hid Day/Week/Month controls and repeated empty-state messages. This pass keeps the saved grid consistent between modes and makes date/view hierarchy explicit.

## Research and design decisions

Research used primary Apple guidance, not a claim that passing code tests validates appearance. These are applications of the guidance to Permagent; Apple does not prescribe this exact card layout.

- [Layout](https://developer.apple.com/design/human-interface-guidelines/layout): align related elements, use grouping to communicate hierarchy, and retain familiar relationships when context changes. Apply one saved grid in viewing/editing modes; date leads and view controls trail on the same header.
- [Segmented controls](https://developer.apple.com/design/human-interface-guidelines/segmented-controls): related choices should have consistent content and balanced widths. Day, Week and Month get equal space; date navigation is a separate control group.
- [Tab views](https://developer.apple.com/design/human-interface-guidelines/tab-views): visible short labels make related content reachable directly. Full-calendar tabs represent the visible pane. Compact agenda launch controls must not falsely imply that a Week/Month pane is already visible.
- [Design principles](https://developer.apple.com/design/human-interface-guidelines/design-principles/): prioritize clear hierarchy, concise wording and iterative visual refinement. Remove the decorative centered date tile and repeated empty-state copy; preserve useful source/permission explanations.
- [Dark Mode](https://developer.apple.com/design/human-interface-guidelines/dark-mode) and [Color](https://developer.apple.com/design/human-interface-guidelines/color): support both appearances and sufficient contrast; do not communicate meaning only through color. Use readable semantic foregrounds and explicit labels/selection states.
- [Scroll views](https://developer.apple.com/design/human-interface-guidelines/scroll-views): make overflow discoverable and avoid unnecessary nested scrolling. Keep the compact header fixed while allowing unusually long agenda/access content to remain reachable.
- [Calendar settings](https://support.apple.com/en-gb/guide/calendar/icl26668/16.0/mac/26): preserve locale/first-day and day/week/month semantics already implemented; this is a presentation change, not a change to calendar data or access.

## Implementation and verification

Home removes the mode-dependent content measurement/masonry pass and uses saved grid dimensions in both modes. Daily Home retains the same width, section headings and responsive columns in Customize and Done. Individual card focus remains separate.

The initial Home-only integrated suite passed 568 files / 4,658 tests. Calendar-focused tests passed 4 files / 55 tests, and final integrated TypeScript passed. The final combined UI suite passed 568 files / 4,663 tests in 136.30 seconds (`home-grid-calendar-full-ui-20261018.log`); TypeScript and `git diff --check` passed. New tests exercise mode transitions and no unintended persistence, not just static styles. An opt-in synthetic `home-review.html?geometry=1` probe compares rendered card coordinates and sizes, relative to the grid, across the actual mode toggle. Browser review is separate from JSDOM evidence.

Rendered review used isolated synthetic data in Chrome Guest, with a Safari cross-check. At 1280px dark appearance, Calendar and To-dos retained 568×288px rectangles and the two Activity cards retained 568×212px rectangles. At 480px silver appearance, cards stacked at 426px wide with the same respective heights. The DOM geometry probe passed both Customize and Done at each width (coordinates compared relative to the grid). Sections remained aligned; no mode-dependent shrink or reflow occurred.

Calendar's compact date and three equal-width launch controls fit on one row at both reviewed widths. Each Day/Week/Month launcher opened the matching selected full view; collapsing returned to the unchanged Home geometry. A populated synthetic Day event opened the correct detail popover and closed successfully. Narrow Week explicitly exposed horizontal scrolling without overflowing the page. Empty dark and populated silver states were visually inspected. Compact source/access content remains scrollable below the fixed header. Essential muted text measures 5.07:1 on the dark surface and 7.56:1 on silver; the old dim foreground measured 2.60:1 on dark, so it is not used for the date/status summary.

Regression coverage includes saved-grid mode parity, focus/overview transitions without persistence, all three Calendar launchers, keyboard handoff, no backend writes on view selection, one empty summary, access disclosure, scroll containment, and unique tab/panel relationships. Existing range/navigation/event/access tests remain in the focused gate.

No Calendar event writes or permission changes were used as test inputs. Native verification and installation remain pending. Review-only browser tabs and server were closed.
