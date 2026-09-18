/**
 * Corrections for daemon-supplied card notes that ask the user to do something
 * the app gives them no way to do.
 *
 * The case this exists for: the Calendar card's entire body read **"Grant
 * Calendar access to see events"**. There is no control anywhere in the shipped
 * app that grants it — no Settings section, no Tauri command, no button on the
 * card. The daemon reads Apple Calendar by sending Apple events to Calendar.app
 * (`osascript` + `tell application "Calendar"`), and macOS refuses that with
 * -1743 and no prompt when the sending process has no
 * NSAppleEventsUsageDescription. Worse, the process macOS holds responsible is
 * `permagentd`, which launchd normally starts as a bare executable with no
 * Info.plist — so on the usual installed setup there is no prompt to raise and
 * nothing for a button to open.
 *
 * An instruction with no way to follow it is the worst of the empty states: it
 * reads as the user's failure to find a control that does not exist. Until a
 * real grant path ships, the card says what is actually true.
 *
 * Deliberately a lookup keyed on the exact daemon string, not a fuzzy match:
 * when the daemon grows a real grant flow and changes its copy, this mapping
 * stops applying on its own rather than masking the new, honest message.
 */

export interface CardNote {
  /** Replacement for the note line. */
  text: string;
  /** Optional second line — the detail that makes the first line actionable. */
  hint?: string;
}

/**
 * Daemon note strings this module rewrites, keyed by `cardType` then by the
 * exact note the endpoint returned.
 */
const CORRECTIONS: Record<string, Record<string, CardNote>> = {
  calendar: {
    // crates/goose-server/src/routes/dashboard_cards.rs — osascript ran and
    // exited non-zero, which in practice means macOS denied the Apple event.
    'Grant Calendar access to see events': {
      text: 'Apple Calendar sync is not wired up yet',
      hint: 'macOS refused Permagent access to Calendar, and this app has no screen that can ask for it — so there is nothing here for you to grant. Meetings you log on a person still show up on this card.',
    },
  },
};

/**
 * Returns the honest replacement for a card note, or `null` when the note the
 * daemon sent is fine as-is (the overwhelmingly common case).
 */
export function correctCardNote(cardType: string, note: string | undefined | null): CardNote | null {
  if (!note) return null;
  return CORRECTIONS[cardType]?.[note] ?? null;
}
