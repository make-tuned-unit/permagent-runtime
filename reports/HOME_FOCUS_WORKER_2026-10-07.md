# Home focus mode + Calendar minimise/expand — worker report (2026-10-07)

Worker: claude-opus-5-5. Worktree `wq-record-20261007`. Scope: `ui/command-center/src/components/dashboard/**` only. No Record, native, api, world or backend changes, no UI control, no cargo, no commits.

## Files touched (copy exactly these)

| File | Status |
|---|---|
| `ui/command-center/src/components/dashboard/homeFocus.tsx` | **new**: focus context, `useHomeFocus`, transition helper, Expand/Collapse controls |
| `ui/command-center/src/components/dashboard/homeFocus.css` | **new**: Dashboard-scoped animation CSS (`pa-home-*` classes, `data-home-focus-vt`) |
| `ui/command-center/src/components/dashboard/Dashboard.tsx` | modified: focus mode wired into the existing grid |
| `ui/command-center/src/components/dashboard/cards/CalendarCard.tsx` | modified: minimised on Home, full when focused, rich date header |
| `ui/command-center/src/components/dashboard/cards/registry.ts` | modified: Calendar size-preset labels and doc only |
| `ui/command-center/src/components/dashboard/Dashboard.focus.test.tsx` | **new**: 11 tests |
| `ui/command-center/src/components/dashboard/cards/CalendarCard.home.test.tsx` | **new**: 6 tests |

## Checks (from `ui/command-center`)

| Command | Exit | Result |
|---|---|---|
| `npx tsc --noEmit -p .` | 0 | no output |
| `npx vitest run src/components/dashboard/Dashboard.focus src/components/dashboard/cards/CalendarCard` | 0 | 4 files, 49/49 passed (11 + 6 new; the existing 18 + 14 Calendar tests unchanged) |
| `npx vitest run src/styles src/components/dashboard` | 1 | 59/60 files, 521/523 tests. The 2 failures are `cards/calendarPermission.test.ts`, which reads **native** `ui/desktop/src-tauri/src/main.rs` and `Cargo.toml`. This worktree's native files predate the integrated calendar code (`calendar_events_in_range` is absent here but present in `world-production-20261006`). They are not caused by this change and should pass in the integration tree. |

I fixed one design-scanner hit before finishing: `textScale.test.ts` flagged a raw 20px in the Calendar date block, which now uses `textSize.title`.

## What the user gets

- **Expand any card.** Every Home card has an **Expand** control (`aria-label="Expand <Card>"`, `aria-expanded="false"`, `aria-controls` = the cell id) in its top-right corner. It appears on hover or focus, is always in the tab order, and is always visible on compact tiles and on devices with no hover.
- **One card in focus.** The focused card is drawn first at full width, with its **full existing body** (no height cap). Above it a focus bar shows "Focused · <Card> · Esc to return" and a **Collapse** button (`aria-label="Collapse <Card>"`, `aria-expanded="true"`). The cell becomes `role="region"`, labelled "<Card>, expanded".
- **The other cards shrink.** They become compact tiles: 3 columns wide (4 to a row), wider wherever `minSpanForWidth` says 3 would be too narrow, so narrow windows get full-width tiles. Each tile is a fixed 3-row height, and the card's own scroll body (fade and "+N more") still works inside it. Every existing action in every card keeps working (tested).
- **One focus at a time.** Expanding another card moves focus to it.
- **Keyboard and focus restore.**
  - Expanding moves keyboard focus to Collapse; collapsing returns it to that card's Expand control.
  - Escape collapses, but only for a key pressed inside Home (or on the page body). It ignores keys from a text field, keys already handled (`defaultPrevented`), and any time a modal (`aria-modal="true"`) is open.
  - It never stops propagation. Calendar's event popover handles its own Escape first, in the capture phase.
- **Animation.**
  - Where View Transitions exist, each cell morphs from its old box to its new one: the focused card grows while the others shrink. The default root slide is switched off for this transition only, so the header doesn't move.
  - Where they don't exist, the focused card fades in using opacity only. A transform would trap a card's fixed-position modal inside the cell.
  - Reduced motion (OS setting or app setting) makes the change instant. The helper skips the transition, and the global guards in `index.css` cover the CSS fallback.
- **The layout is never changed.** Focus is session-only display state; `persistLayout` is never called (tested). Customize turns focus off and hides the Expand controls. Removing the focused card clears focus.
- **Calendar.**
  - **Minimised on Home** at every size:
    - a tear-off date block (weekday and day number);
    - the full date;
    - "N events left today" (says "Calendar not connected" rather than claiming an empty day when no calendar is reachable);
    - the existing Next up strip and today's list.
  - The whole date header is one button: "Expand calendar — <date>, <summary>. Opens Day, Week and Month views."
  - **Focused**, it is the existing full calendar: Day, Week and Month tabs, period navigation and keyboard shortcuts, unchanged.
  - Clicking an event while minimised still opens its details.
  - Outside Home (no focus context) it keeps its old width-based behaviour, and a forced `sizeMode` still wins, so the existing Calendar tests are untouched.

## How it fits the existing layout system

- **Overview unchanged.** The masonry grid (`MASONRY_ROW_UNIT` spans and dense flow) is untouched in the overview.
- **Focus mode uses auto rows.** While a card is focused, the grid switches to `grid-auto-rows: auto`. The focused card is placed full width with CSS `order: -1` (the DOM order and saved layout are unchanged), and the compact tiles have a fixed height. No measured spans are needed.
- **Span measurement pauses.** Measurement stops during focus, and the span settle state is reset when the overview returns. This matters because `settleSpan`'s frozen state only grows: measuring focus-sized cells would have left cards inflated after collapse.
- **Opt-in for cards.** Cards opt into richer behaviour through `useCardPresentation()` (`overview` / `focused` / `compact`, plus `expand` / `collapse`). Only Calendar uses it; every other card works unchanged.

## Tests (behavioural)

**`Dashboard.focus.test.tsx`** (real Dashboard grid; data hooks and unrelated header chrome mocked):
- Expand controls are labelled and collapsed;
- expanding focuses one card and makes the others compact (cards are told their presentation);
- one focus at a time;
- Collapse restores and returns keyboard focus;
- Escape collapses and restores;
- Escape is ignored with a modal open, in a text field, or when already handled;
- card actions work in focus and compact;
- a card can expand itself;
- the layout is never saved;
- Customize turns focus off;
- focus clears when the card leaves the layout.

**`CalendarCard.home.test.tsx`:**
- minimised content (date, "3 events left today", Next up, today's list with past events excluded);
- the header is an `aria-expanded="false"` button that expands;
- an event still opens its popover;
- compact while another card is focused;
- "Calendar not connected" honesty;
- focused shows the Day/Week/Month tabs, the Week view and switching to Month.

## Not verified

- **No real WebKit render.** jsdom has no layout and no View Transitions, so the morph animation, the 3-row tile height at real widths and the hover visibility of the Expand control are unverified. The parent's rebuild and Computer Use pass should check focus/collapse at a wide and a narrow window width, with Reduce Motion on and off.
- **Possible header overlap.** The Expand control sits over each card's top-right corner. On hover it may cover a card header's right-hand text (e.g. a `SectionTitle` "right" label). This needs a visual check.
- **Calendar default changed.** Calendar is now minimised on Home even at the Medium and Large presets; the full calendar is one click away. The preset labels were updated to say so. If the parent prefers wide calendars to stay full in the overview, that is a one-line change in `CalendarCard.tsx` (`home ? home.presentation !== 'focused'`).
