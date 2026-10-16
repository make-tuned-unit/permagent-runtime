# Home redesign

## Diagnosis

The supplied live screenshot supports five concrete findings:

1. The header already shows connection status, then a full-width “Henry is ready” card repeats readiness before any useful work.
2. “Needs you: 45” and “Today's Focus” both point to the same Decision Inbox. Focus repeats the queue total in each explanation and “Next up” adds more candidates. The page looks like several competing priority systems.
3. Inconsistent headings (uppercase labels, large count, sentence-case card titles, standalone calendar date) and mixed 5/7/full-width/three-column sizes make unrelated information look equally important.
4. Short content sits inside a much larger layout footprint. The full-width In flight card has one small item; a large empty band separates it from the calendar. Dense grid placement, fixed caps, and measured masonry spans layer several sizing rules together.
5. Multiple cyan review links compete, while Calendar and To-dos begin near the bottom of the viewport. Overnight repeats new decisions and blockers that are already represented above.

Source inspection also finds optional banners before the grid and broad analytics/history cards in the fallback default. The app's existing ViewHeader, surface/border tokens, shared Button, SectionTitle and focus controls supply the design language. The resumed read-only native review and synthetic browser checks are documented in `APP_UX_REVIEW_2026-10-15.md` and `verification.md`, superseding the initial crawl limitation.

## Chosen architecture

Home is a daily summary with three levels: Decisions → Today → Activity. Five default cards, one pending queue, one prominent review action. Status stays in the header. Calendar and To-dos share a row; In flight and Recent conversations share a second row. Use the existing Recent conversations card for history. Overnight and Today's Focus remain optional cards for existing custom layouts.

```text
Home  • connection                     …  Layouts  Customize

┌ Decisions ─────────────────────────────────── 45 waiting ┐
│ Actual first request, wrapping to remain readable         │
│ Oldest waiting … / blocked-goal count when present         │
│ [ Review decisions ]                                      │
└───────────────────────────────────────────────────────────┘
Today
┌ Calendar ─────────────────┐ ┌ To-dos ────────────────────┐
│ Date / next appointment    │ │ Overdue / today / upcoming │
│ Today's events             │ │ Dated tasks                │
└────────────────────────────┘ └────────────────────────────┘
Activity
┌ In flight ────────────────┐ ┌ Recent conversations ──────┐
│ Current goals / state      │ │ Past conversations         │
└────────────────────────────┘ └────────────────────────────┘
Optional learning / resurfaced-memory suggestion
```

At narrow content widths the same order becomes a single column. The default uses normal content-sized CSS grid rows, no dense backfilling or measured row reservations. Peer cards share column widths and top edges; empty cards do not stretch just to fill the tallest neighbor. Long lists retain bounded scrolling and expansion. The page uses a bounded reading width on large displays.

## Compatibility and state

Keep Layouts, template previews, Customize, reset, revert, drag, resize and card expansion. The Morning Briefing template becomes this daily Home and keeps its stable template ID. A server-provided old Morning Briefing definition must not replace the new frontend default in the gallery.

Recognize only exact shipped defaults for display upgrades. Do not infer “default” from the card type set alone: resized, hidden, reordered, renamed or extended layouts are user choices. Upgrade the exact old fallback and Morning Briefing without writing the live layout on read. Loading no longer auto-adds or resizes cards or writes a layout. The exact old defaults are upgraded for display, and retired readiness cards are still filtered out. Custom layouts otherwise retain their arrangements and optional cards. Council and Growth remain available through Customize.

## States and interactions

- Pending: one count, one concrete first headline from the same decision response, oldest age, optional blocked-goal count, and Review decisions.
- Zero decisions: “No decisions waiting”; blocked goals remain explicit. Ordinary Open inbox link still reaches the full review surface.
- Loading: Checking decisions; no fake zero.
- Error: Couldn't load decisions and Retry, never an all-clear claim.
- Recent conversations: only recorded conversations from the existing recent-session feed; no repeated decisions or stalled goals. Do not imply confirmed completion or a local-day window.
- New profile: retain the existing model setup entry point and hide unused default cards as before.

## Verification plan

TypeScript; focused dashboard tests; full suite once. Extend tests for default compatibility, template precedence, zero/error decision states, review entry points, default reading order and custom layout preservation. Use a fixture-only browser harness if available; it must never contact the live daemon or present its screenshots as live data. Record results and remaining visual limitations in verification.md.

## Activity data constraint

Source verification of `get_dashboard_insights` found its “finished” bucket selects user/scheduled sessions idle for two minutes since UTC midnight. That is not evidence of successful completion or the user’s local day. The default therefore uses the existing `recent` card, labelled “Recent conversations”, with per-item ages and no “last 24h” promise. The optional Overnight card is retained unchanged.
