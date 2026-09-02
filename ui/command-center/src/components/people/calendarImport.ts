/**
 * People's calendar auto-import, said out loud.
 *
 * The import fires on every mount of the People tab. It reads the user's
 * macOS Calendar — personal data, without being asked — and until now it was
 * silent in *both* branches: a successful import silently grew the directory,
 * and a failure was swallowed by a bare `catch`. Nothing on screen ever said
 * this had happened, so the meetings on a person's profile arrived from
 * nowhere and a broken import was indistinguishable from a quiet week.
 *
 * What the UI may honestly claim is bounded by what the daemon actually
 * reports. `import_matching_events` is documented best-effort: a permission
 * failure returns 0, exactly like a genuinely empty window. So this file does
 * NOT invent a "Calendar access is off" state — it says what is known (an
 * import ran, and what it found) and names the ambiguity in the hover text
 * rather than guessing at it. A request that actually fails is a different
 * thing and says so, with a way to try again.
 */

export type CalendarImportPhase =
  | { phase: 'importing' }
  | { phase: 'done'; imported: number; at: number }
  | { phase: 'failed'; message: string };

export interface CalendarImportLine {
  text: string;
  /** `muted` is the resting caption; `warning` is a failure the user can act on. */
  tone: 'muted' | 'warning';
  /** Longer explanation for the hover, where there is one worth saying. */
  title?: string;
  /** Whether the line should offer Retry. */
  retry: boolean;
}

/** Ambiguity worth stating once, in the place it applies. */
const ZERO_TITLE =
  'Meetings are matched by full name in the event title. A calendar Permagent has not been '
  + 'granted access to reads the same as an empty one, so if you expected meetings here, check '
  + 'macOS Settings → Privacy & Security → Calendars.';

export function calendarImportLine(state: CalendarImportPhase): CalendarImportLine {
  if (state.phase === 'importing') {
    return { text: 'Checking your calendar…', tone: 'muted', retry: false };
  }
  if (state.phase === 'failed') {
    return {
      text: `Couldn't check your calendar — ${state.message}`,
      tone: 'warning',
      retry: true,
    };
  }
  if (state.imported === 0) {
    return {
      text: 'Calendar checked · no new meetings',
      tone: 'muted',
      title: ZERO_TITLE,
      retry: false,
    };
  }
  return {
    text: `Calendar synced · ${state.imported} new meeting${state.imported === 1 ? '' : 's'}`,
    tone: 'muted',
    retry: false,
  };
}
