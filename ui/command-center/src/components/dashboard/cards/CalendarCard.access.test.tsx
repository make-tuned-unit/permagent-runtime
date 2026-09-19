/**
 * @vitest-environment jsdom
 *
 * The Home Calendar card's grant path, end to end through the renderer.
 *
 * The bug: "i cant find anywhere to grant access to my iCal so my calendar can
 * sync". The card described a permission and offered no control anywhere to
 * grant it. These tests drive the real ManifestCard with the Tauri `invoke`
 * mocked at the module boundary and pin that:
 *   - notDetermined shows a Connect Calendar button, and pressing it calls the
 *     shell's `request_calendar_access` and then lists the Apple events;
 *   - denied shows "Calendar access is off — open System Settings" and the
 *     button calls `open_calendar_privacy_settings`;
 *   - writeOnly (where the installed app actually sat) offers Allow Full
 *     Access, which asks the shell for the upgrade prompt; only if macOS
 *     answers write-only again does it fall back to Open System Settings;
 *   - fullAccess lists Apple events AND the People meetings the daemon still
 *     returns, without the daemon's stale grant note;
 *   - outside Tauri the card says it needs the desktop app.
 *
 * What this does NOT prove: that EventKit really prompts, that macOS lists
 * Permagent under Privacy › Calendars, or that the command names match the
 * Rust registrations at runtime. The names are pinned against main.rs and
 * calendar.rs in `calendarPermission.test.ts` (node env, where the Rust
 * sources can be read); the prompt itself needs the signed app on a real Mac.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { ManifestCard } from './ManifestCard';
import type { CardManifest } from './registry';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** The exact string `dashboard_cards.rs` returns when osascript is refused. */
const DAEMON_GRANT_NOTE = 'Grant Calendar access to see events';

const calendarManifest: CardManifest = {
  type: 'calendar',
  name: 'Calendar',
  description: "Today's events",
  defaultSize: { w: 5, h: 4 },
  layout: 'list',
  dataEndpoint: '/api/dashboard/calendar',
  refreshSeconds: 300,
  source: 'built-in',
};

function at(h: number, m = 0): number {
  const d = new Date();
  d.setHours(h, m, 0, 0);
  return d.getTime();
}

const EVENTS = [
  { title: 'Design review', startMs: at(11), endMs: at(12), allDay: false, calendar: 'Work', location: null },
];

let container: HTMLDivElement;
let root: Root;

/** Mutable per-test shell state the invoke mock reads. */
let shellStatus = 'notDetermined';
let grantOnRequest = 'fullAccess';

beforeEach(() => {
  apiFetch.mockReset();
  invoke.mockReset();
  shellStatus = 'notDetermined';
  grantOnRequest = 'fullAccess';
  invoke.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case 'calendar_authorization_status': return shellStatus;
      case 'request_calendar_access': shellStatus = grantOnRequest; return shellStatus;
      case 'calendar_upcoming_events':
        if (shellStatus !== 'fullAccess') throw new Error('Calendar access is ' + shellStatus);
        return EVENTS;
      case 'open_calendar_privacy_settings': return null;
      default: throw new Error('unexpected command ' + cmd);
    }
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
});

function inDesktopApp() {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
}

/** Dynamic import + several awaited invokes: let them all land. */
async function settle() {
  for (let i = 0; i < 6; i++) {
    await act(async () => { await new Promise(r => setTimeout(r, 0)); });
  }
}

async function render() {
  await act(async () => { root.render(<ManifestCard manifest={calendarManifest} />); });
  await settle();
}

function button(label: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll('button')).find(b => b.textContent === label);
}

describe('Calendar card — Apple Calendar access', () => {
  it('offers Connect Calendar when access was never asked, and lists events once granted', async () => {
    inDesktopApp();
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    const connect = button('Connect Calendar');
    expect(connect).toBeDefined();
    expect(container.textContent).not.toContain(DAEMON_GRANT_NOTE);

    await act(async () => { connect!.click(); });
    await settle();

    expect(invoke).toHaveBeenCalledWith('request_calendar_access', undefined);
    expect(invoke).toHaveBeenCalledWith('calendar_upcoming_events', { days: 1 });
    expect(container.textContent).toContain('Design review');
    expect(button('Connect Calendar')).toBeUndefined();
  });

  it('when denied, says so and opens the Privacy › Calendars pane', async () => {
    inDesktopApp();
    shellStatus = 'denied';
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    expect(container.textContent).toContain('Calendar access is off — open System Settings');
    const open = button('Open System Settings');
    expect(open).toBeDefined();
    await act(async () => { open!.click(); });
    await settle();
    expect(invoke).toHaveBeenCalledWith('open_calendar_privacy_settings', undefined);
    expect(invoke).not.toHaveBeenCalledWith('calendar_upcoming_events', expect.anything());
  });

  it('from write-only, asks macOS for full access and lists events once upgraded', async () => {
    inDesktopApp();
    shellStatus = 'writeOnly';
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    expect(container.textContent).toContain('Permagent can add events but not read them');
    // Before the fix the only control here was Open System Settings, and the
    // shell's request returned early without prompting for any non-first ask.
    expect(button('Open System Settings')).toBeUndefined();
    const allow = button('Allow Full Access');
    expect(allow).toBeDefined();

    await act(async () => { allow!.click(); });
    await settle();

    expect(invoke).toHaveBeenCalledWith('request_calendar_access', undefined);
    expect(container.textContent).toContain('Design review');
    expect(container.querySelector('[data-testid="calendar-access"]')).toBeNull();
  });

  it('from write-only, falls back to System Settings when macOS keeps it write-only', async () => {
    inDesktopApp();
    shellStatus = 'writeOnly';
    grantOnRequest = 'writeOnly';
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    await act(async () => { button('Allow Full Access')!.click(); });
    await settle();

    expect(button('Allow Full Access')).toBeUndefined();
    const open = button('Open System Settings');
    expect(open).toBeDefined();
    await act(async () => { open!.click(); });
    await settle();
    expect(invoke).toHaveBeenCalledWith('open_calendar_privacy_settings', undefined);
    expect(invoke).not.toHaveBeenCalledWith('calendar_upcoming_events', expect.anything());
  });

  it('from write-only, keeps offering Allow Full Access when the prompt timed out unanswered', async () => {
    // QA's screen was locked, so the upgrade prompt sat unanswered until the
    // shell gave up. The shell used to report the unchanged writeOnly then,
    // and the card retired Allow Full Access for Open System Settings as if
    // macOS had declined. It now reports `timedOut`, which must not count as
    // an answer. Proves the renderer's handling of that value; that the Rust
    // side returns it is pinned in calendarPermission.test.ts.
    inDesktopApp();
    shellStatus = 'writeOnly';
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();
    invoke.mockImplementation(async (cmd: string) => {
      switch (cmd) {
        case 'calendar_authorization_status': return shellStatus;
        case 'request_calendar_access': return 'timedOut';
        default: throw new Error('unexpected command ' + cmd);
      }
    });

    await act(async () => { button('Allow Full Access')!.click(); });
    await settle();

    expect(invoke).toHaveBeenCalledWith('request_calendar_access', undefined);
    // The status is read again after the timeout, so a late answer still lands.
    expect(invoke.mock.calls.filter(c => c[0] === 'calendar_authorization_status').length).toBeGreaterThan(1);
    expect(container.textContent).toContain('Permagent can add events but not read them');
    expect(button('Open System Settings')).toBeUndefined();
    expect(button('Allow Full Access')).toBeDefined();
    expect(container.textContent).not.toContain('not available on this computer');
  });

  it('when authorized, lists Apple events together with meetings logged on people', async () => {
    inDesktopApp();
    shellStatus = 'fullAccess';
    apiFetch.mockResolvedValue({
      cells: [{ label: 'Coffee with Priya', value: 'Priya Shah', sub: '9:30 AM' }],
      note: DAEMON_GRANT_NOTE,
    });
    await render();

    const text = container.textContent ?? '';
    expect(text).toContain('Design review');
    expect(text).toContain('Coffee with Priya');
    expect(text).toContain('Priya Shah');
    expect(text).not.toContain(DAEMON_GRANT_NOTE);
    expect(container.querySelector('[data-testid="calendar-access"]')).toBeNull();
    // Time order: the 9:30 People meeting before the 11:00 Apple event.
    expect(text.indexOf('Coffee with Priya')).toBeLessThan(text.indexOf('Design review'));
  });

  it('keeps the Connect button under People meetings when Apple Calendar is not connected', async () => {
    inDesktopApp();
    apiFetch.mockResolvedValue({
      cells: [{ label: 'Coffee with Priya', value: 'Priya Shah', sub: '9:30 AM' }],
      note: DAEMON_GRANT_NOTE,
    });
    await render();

    expect(container.textContent).toContain('Coffee with Priya');
    expect(button('Connect Calendar')).toBeDefined();
  });

  // The daemon's calendar endpoint runs two osascript reads (eight seconds
  // each) before replying, and the card used to show "Loading…" for all of
  // it, hiding the Connect button the shell had already answered for. A
  // never-settling apiFetch stands in for that slow endpoint.
  it('shows Connect Calendar while the daemon endpoint is still loading', async () => {
    inDesktopApp();
    apiFetch.mockReturnValue(new Promise(() => {}));
    await render();

    expect(container.textContent).not.toContain('Loading…');
    expect(button('Connect Calendar')).toBeDefined();
  });

  it('lists Apple events while the daemon endpoint is still loading', async () => {
    inDesktopApp();
    shellStatus = 'fullAccess';
    apiFetch.mockReturnValue(new Promise(() => {}));
    await render();

    expect(container.textContent).not.toContain('Loading…');
    expect(container.textContent).toContain('Design review');
  });

  it('with access and no Apple events, waits for People before saying the day is empty', async () => {
    inDesktopApp();
    shellStatus = 'fullAccess';
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'calendar_authorization_status') return 'fullAccess';
      if (cmd === 'calendar_upcoming_events') return [];
      throw new Error('unexpected command ' + cmd);
    });
    apiFetch.mockReturnValue(new Promise(() => {}));
    await render();

    expect(container.textContent).toContain('Loading…');
    expect(container.textContent).not.toContain('No events today');
  });

  // A Permagent.app built from 6b438fde carries calendar.rs but registers none
  // of its commands, so Tauri rejects every calendar invoke. The card blamed
  // the Mac ("not available on this computer") for what an update fixes.
  it('when the app does not register the calendar commands, says to update Permagent', async () => {
    inDesktopApp();
    invoke.mockImplementation(async (cmd: string) => {
      throw new Error(`Command ${cmd} not found`);
    });
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    const panel = container.querySelector('[data-testid="calendar-access"]');
    expect(panel?.getAttribute('data-state')).toBe('shell-outdated');
    expect(container.textContent).toContain('Update Permagent to connect Apple Calendar');
    expect(container.textContent).not.toContain('Apple Calendar is not available on this computer');
    expect(container.textContent).not.toContain(DAEMON_GRANT_NOTE);
    // No button that would only invoke another missing command.
    expect(container.querySelector('button')).toBeNull();
  });

  it('when the app does not register the calendar commands, still shows People meetings', async () => {
    inDesktopApp();
    invoke.mockRejectedValue(new Error('Command calendar_authorization_status not found'));
    apiFetch.mockResolvedValue({
      cells: [{ label: 'Coffee with Priya', value: 'Priya Shah', sub: '9:30 AM' }],
      note: DAEMON_GRANT_NOTE,
    });
    await render();

    expect(container.textContent).toContain('Coffee with Priya');
    expect(container.textContent).toContain('Update Permagent to connect Apple Calendar');
  });

  it('outside the desktop app says it needs the desktop app and invokes nothing', async () => {
    apiFetch.mockResolvedValue({ cells: [], note: DAEMON_GRANT_NOTE });
    await render();

    expect(container.textContent).toContain('Apple Calendar connects in the Permagent desktop app');
    expect(container.querySelector('button')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
  });

  it('leaves non-calendar cards alone: no shell calls, no access line', async () => {
    inDesktopApp();
    apiFetch.mockResolvedValue({ cells: [{ label: 'CPU load', value: '42%' }] });
    await act(async () => {
      root.render(<ManifestCard manifest={{ ...calendarManifest, type: 'system_stats', name: 'System', layout: 'stat-grid' }} />);
    });
    await settle();
    expect(invoke).not.toHaveBeenCalled();
    expect(container.querySelector('[data-testid="calendar-access"]')).toBeNull();
  });
});
