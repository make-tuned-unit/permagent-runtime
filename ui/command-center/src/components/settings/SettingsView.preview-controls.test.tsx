/** @vitest-environment jsdom
 *
 * 2026-08 finish-the-settings ruling: every preview-only control was either
 * wired to real state or removed. These tests pin the new truth — the panes
 * that used to carry disabled mockup controls now render NO disabled
 * preview controls at all. (The deliberately-locked "Ask every time" /
 * "Smart approve" trust modes in Autonomy are a separate, honest guard and
 * are covered by SettingsView.autonomy.test.tsx.)
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getCrashConsent: vi.fn(() => new Promise(() => {})),
    setCrashConsent: vi.fn(),
    setAnalyticsConsent: vi.fn(),
    exportCrashReport: vi.fn(),
  },
  apiFetch: vi.fn(),
}));

vi.mock('../../lib/notifications', () => ({
  KIND_LABELS: {},
  getNotificationPrefs: vi.fn(() => ({})),
  setNotificationPref: vi.fn(),
  getOsNotificationsEnabled: vi.fn(() => false),
  setOsNotificationsEnabled: vi.fn(),
}));

import { DataPanel, MemoryPanel, PreferencesPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(panel: React.ReactNode) {
  await act(async () => { root.render(panel); });
}

function disabledControls(): HTMLElement[] {
  return Array.from(container.querySelectorAll('button:disabled, input:disabled, select:disabled'));
}

describe('settings preview controls are gone', () => {
  it('Preferences has a LIVE open-on-launch select and no disabled preview controls', async () => {
    await render(<PreferencesPanel />);
    const select = container.querySelector('select') as HTMLSelectElement;
    expect(select).toBeTruthy();
    expect(select.disabled).toBe(false);
    expect(disabledControls()).toHaveLength(0);
  });

  it('Memory keeps only the live Brain/Librarian links — no sliders, no toggles', async () => {
    await render(<MemoryPanel />);
    expect(container.querySelectorAll('input[type="range"]')).toHaveLength(0);
    expect(disabledControls()).toHaveLength(0);
    expect(container.textContent).toContain('Open Brain view');
  });

  it('Data & privacy dropped the fake local-first/E2E/diagnostics/prompt toggles', async () => {
    await render(<DataPanel />);
    expect(disabledControls()).toHaveLength(0);
    for (const gone of [
      'Keep everything on this device',
      'End-to-end encryption',
      'Share anonymous diagnostics',
      'Share prompts to improve models',
    ]) {
      expect(container.textContent).not.toContain(gone);
    }
    // The real controls remain: analytics consent + the redacted export.
    expect(container.textContent).toContain('Share product analytics');
    expect(container.textContent).toContain('Export redacted crash report');
  });
});
