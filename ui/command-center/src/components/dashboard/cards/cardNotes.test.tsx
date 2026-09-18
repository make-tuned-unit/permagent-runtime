/**
 * @vitest-environment jsdom
 *
 * The Calendar card's honest empty state.
 *
 * The bug: the Calendar card's whole body read "Grant Calendar access to see
 * events" while the app shipped no control that grants it — no Settings
 * section, no Tauri command, no button. These tests pin that the card no
 * longer instructs the user to do something impossible, and that the
 * correction is keyed on the exact daemon string — so a future daemon with a
 * real grant flow takes the card back over on its own.
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

import { correctCardNote } from './cardNotes';
import { ManifestCard } from './ManifestCard';
import type { CardManifest } from './registry';

/** The exact string `dashboard_cards.rs` returns when osascript is refused. */
const DAEMON_GRANT_NOTE = 'Grant Calendar access to see events';

describe('correctCardNote', () => {
  it('rewrites the calendar grant-access note, which nothing in the app can act on', () => {
    const note = correctCardNote('calendar', DAEMON_GRANT_NOTE);
    expect(note).not.toBeNull();
    // The replacement must not tell the user to grant anything.
    expect(note!.text.toLowerCase()).not.toContain('grant');
    expect(note!.hint).toBeTruthy();
  });

  it('leaves every other note alone', () => {
    expect(correctCardNote('calendar', 'Calendar is unavailable right now')).toBeNull();
    expect(correctCardNote('calendar', 'No events today')).toBeNull();
    expect(correctCardNote('weather', DAEMON_GRANT_NOTE)).toBeNull();
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
  it('does not ask the user to grant access there is no way to grant', async () => {
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await act(async () => { root.render(<ManifestCard manifest={calendarManifest} />); });
    await flush();

    const text = container.textContent ?? '';
    expect(text).not.toContain(DAEMON_GRANT_NOTE);
    expect(text).toContain('Apple Calendar sync is not wired up yet');
    // The second line says why, so the state is diagnosable rather than blank.
    expect(text).toContain('nothing here for you to grant');
  });

  it('still shows an unmodified daemon note verbatim', async () => {
    apiFetch.mockResolvedValue({ cells: [], note: 'Calendar is unavailable right now' });
    await act(async () => { root.render(<ManifestCard manifest={calendarManifest} />); });
    await flush();

    expect(container.textContent).toContain('Calendar is unavailable right now');
  });
});
