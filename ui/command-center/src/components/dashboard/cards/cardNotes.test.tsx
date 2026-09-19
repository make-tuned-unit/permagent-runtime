/**
 * @vitest-environment jsdom
 *
 * The Calendar card's honest empty state.
 *
 * The bug: the Calendar card's whole body read "Grant Calendar access to see
 * events" while the app shipped no control that grants it ("i cant find
 * anywhere to grant access to my iCal"). The desktop shell now asks EventKit
 * itself, so the daemon's note is superseded: the card drops it and shows the
 * shell's live access state instead (pinned in CalendarCard.access.test.tsx).
 * These tests pin that the stale instruction never reaches the screen, and
 * that the match is keyed on the exact daemon string — so a future daemon
 * message is shown as-is rather than silently swallowed.
 *
 * The macOS keys that make a prompt possible at all are pinned next door, in
 * `calendarPermission.test.ts` (node env — this file is jsdom, where
 * `import.meta.url` is an http URL and cannot be read from disk).
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

import { correctCardNote, isSupersededCardNote } from './cardNotes';
import { ManifestCard } from './ManifestCard';
import type { CardManifest } from './registry';

/** The exact string `dashboard_cards.rs` returns when osascript is refused. */
const DAEMON_GRANT_NOTE = 'Grant Calendar access to see events';

describe('isSupersededCardNote', () => {
  it('marks the calendar grant-access note as answered by the shell', () => {
    expect(isSupersededCardNote('calendar', DAEMON_GRANT_NOTE)).toBe(true);
  });

  it('matches only that exact string on that card', () => {
    expect(isSupersededCardNote('calendar', 'Calendar is unavailable right now')).toBe(false);
    expect(isSupersededCardNote('weather', DAEMON_GRANT_NOTE)).toBe(false);
    expect(isSupersededCardNote('calendar', undefined)).toBe(false);
  });
});

describe('correctCardNote', () => {
  it('no longer rewrites the calendar grant note into other words', () => {
    expect(correctCardNote('calendar', DAEMON_GRANT_NOTE)).toBeNull();
  });

  it('leaves every other note alone', () => {
    expect(correctCardNote('calendar', 'Calendar is unavailable right now')).toBeNull();
    expect(correctCardNote('calendar', 'No events today')).toBeNull();
    expect(correctCardNote('calendar', undefined)).toBeNull();
    expect(correctCardNote('calendar', '')).toBeNull();
  });
});

// ── The card as rendered ─────────────────────────────────────────────────────

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

const calendarManifest: CardManifest = {
  type: 'calendar',
  name: 'Calendar',
  description: "Today's events",
  defaultSize: { w: 5, h: 4 },
  layout: 'list',
  dataEndpoint: '/api/dashboard/calendar',
  source: 'built-in',
};

describe('Calendar card empty state', () => {
  it('in a browser tab, replaces the grant note with where the grant actually lives', async () => {
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await act(async () => { root.render(<ManifestCard manifest={calendarManifest} />); });
    await flush();

    const text = container.textContent ?? '';
    expect(text).not.toContain(DAEMON_GRANT_NOTE);
    expect(text).toContain('Apple Calendar connects in the Permagent desktop app');
    // Says that People meetings are unaffected, so the state is not a dead end.
    expect(text).toContain('Meetings you log on a person still show up');
  });

  it('still shows an unmodified daemon note verbatim on other cards', async () => {
    apiFetch.mockResolvedValue({ cells: [], note: 'Weather is unavailable right now' });
    await act(async () => {
      root.render(<ManifestCard manifest={{ ...calendarManifest, type: 'weather', name: 'Weather' }} />);
    });
    await flush();

    expect(container.textContent).toContain('Weather is unavailable right now');
  });
});
