/**
 * Corrections for daemon-supplied card notes that ask the user to do something
 * the app gives them no way to do — and the notes the app shell now answers
 * better than the daemon can.
 *
 * The case this exists for: the Calendar card's entire body read **"Grant
 * Calendar access to see events"** while the app shipped no control that
 * grants it, and the user reported "i cant find anywhere to grant access to my
 * iCal so my calendar can sync". The daemon reads Apple Calendar by sending
 * Apple events to Calendar.app (`osascript` + `tell application "Calendar"`),
 * and macOS refuses that with -1743 and no prompt — the process it holds
 * responsible is `permagentd`, which launchd normally starts as a bare
 * executable with no Info.plist, so there is no prompt to raise.
 *
 * The grant path now lives in the signed desktop app (EventKit, in
 * `src-tauri/src/calendar.rs`), and the Calendar card draws its live state —
 * Connect Calendar, Open System Settings, or "needs the desktop app" — from
 * `calendarAccess.ts`. So the daemon's grant note is no longer rewritten into
 * different words; it is superseded, and the card shows the shell's answer in
 * its place. Printing both would leave the stale instruction on screen next to
 * the button that replaced it.
 *
 * Deliberately a lookup keyed on the exact daemon string, not a fuzzy match:
 * when the daemon changes its copy, this mapping stops applying on its own
 * rather than masking the new message.
 */

export interface CardNote {
  /** Replacement for the note line. */
  text: string;
  /** Optional second line — the detail that makes the first line actionable. */
  hint?: string;
}

/**
 * Daemon note strings this module rewrites, keyed by `cardType` then by the
 * exact note the endpoint returned. Empty today: the one entry it held (the
 * calendar grant note) is now superseded instead — see `SUPERSEDED`.
 */
const CORRECTIONS: Record<string, Record<string, CardNote>> = {};

/**
 * Daemon notes about something the desktop shell now owns, keyed by
 * `cardType`. The card drops these and renders the shell's live state.
 */
const SUPERSEDED: Record<string, ReadonlySet<string>> = {
  calendar: new Set([
    // crates/goose-server/src/routes/dashboard_cards.rs — osascript ran and
    // exited non-zero, which in practice means macOS denied the Apple event.
    // The EventKit access line on the card (CalendarAccessPanel) says what is
    // actually true and offers the control that fixes it.
    'Grant Calendar access to see events',
  ]),
};

/**
 * Returns the honest replacement for a card note, or `null` when the note the
 * daemon sent is fine as-is (the overwhelmingly common case).
 */
export function correctCardNote(cardType: string, note: string | undefined | null): CardNote | null {
  if (!note) return null;
  return CORRECTIONS[cardType]?.[note] ?? null;
}

/**
 * True when the daemon's note is about a permission the app shell now asks
 * for itself, so the card should show the shell's state instead of the note.
 */
export function isSupersededCardNote(cardType: string, note: string | undefined | null): boolean {
  if (!note) return false;
  return SUPERSEDED[cardType]?.has(note) ?? false;
}
