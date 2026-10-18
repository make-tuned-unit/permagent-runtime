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

No Calendar event writes or permission changes were used as test inputs. Review-only browser tabs and server were closed. Native verification passed as recorded below.


## Packaged release

App/UI source: `3c1c9c4e93cf3b0a011ad27d25133c070fadfaf1`; unchanged verified backend: `227484e71d44e04e80187f8956f371ca1f6d0de4`. The guarded Tauri build, artifact directory creation and artifact copy all exited 0 (`20261018-194658-80693`, `20261018-195725-94594`, `20261018-195726-94621`). Normal bundle verification passed deep/strict signatures, selected Developer ID, source/code provenance, resources and 332 UI file hashes. Main executable SHA-256: `14963b4ce8636996b712aa421af9d8763c099790536fbcc26e8d60151f85b580`. Exact artifact and receipts are in [desktop-rebuild-manifest.json](desktop-rebuild-manifest.json).

Two external-volume `permagentd --version` probes timed out at 30 seconds (one during bundle verification, one afterward). A sample of the second process showed 112 KiB footprint and `_dyld_start`, without application frames. Root cause is unproven; the source was not changed to address it. The verified local staged copy then passed both isolated bundled version probes: daemon in 5.65 seconds, CLI in 0.04 seconds, each version 1.31.0. [Version smoke receipt](home-grid-calendar-version-smoke.json) preserves the initial failures and final success. No security protections were changed.

The installation staging copy passed the existing disk/lock/watchdog guard and exact signature/resource/hash validation. [Installation receipt](home-grid-calendar-install-receipt.json) records the preserved baseline, new source and rollback location.


## Installed native verification

Installed at `/Applications/Permagent.app` on October 18 at 20:04, with deep/strict signatures and exact executable/resource hashes verified. The existing LaunchAgent configuration is unchanged; the installed daemon returned HTTP 200 and expected backend version 227484e71. Previous app retained at `/Applications/.Permagent-rollback-20261018-200312.app`, with its executable hashes matched to the preinstall baseline.

Native Home loaded with the hub connected. Visual comparison across Customize and Done retained equal Calendar/To-dos card heights and the saved grid geometry. The compact date appeared at left with Day/Week/Month at right. Each launcher opened the matching selected full calendar pane; Month rendered existing calendar events. The full header retained the left period title, right view selector, and separate period navigation. Collapse returned to Home. The app remains open on Home. No live calendar edits, permission changes, outbound messages or model requests were exercised.

D29–D31 are complete within these acceptance criteria. The unchanged backend was reused rather than rebuilt/retested; backend-wide evidence remains in the previous functional audit. The two external-volume startup timeouts remain documented, although staged startup and installed runtime checks passed.
