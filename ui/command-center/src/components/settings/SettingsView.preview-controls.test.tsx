/** @vitest-environment jsdom */

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

import { DataPanel, MemoryPanel, PreferencesPanel, ProfilePanel } from './SettingsView';

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

describe('settings preview controls', () => {
  it('disables every preview-only profile, preference, and memory control', async () => {
    await render(<ProfilePanel />);
    const profileInputs = Array.from(container.querySelectorAll('input')) as HTMLInputElement[];
    expect(profileInputs).toHaveLength(3);
    expect(profileInputs.every(control => control.disabled)).toBe(true);

    await render(<PreferencesPanel />);
    expect((container.querySelector('select') as HTMLSelectElement).disabled).toBe(true);
    expect(container.querySelectorAll('button:disabled')).toHaveLength(4);

    await render(<MemoryPanel />);
    const sliders = Array.from(container.querySelectorAll('input[type="range"]')) as HTMLInputElement[];
    expect(sliders).toHaveLength(2);
    expect(sliders.every(control => control.disabled)).toBe(true);
    expect(container.querySelectorAll('button:disabled')).toHaveLength(5);
  });

  it('disables local-only and encryption preview toggles', async () => {
    await render(<DataPanel />);
    const labels = ['Keep everything on this device', 'End-to-end encryption'];
    for (const text of labels) {
      const label = Array.from(container.querySelectorAll('div')).find(node => node.textContent === text);
      const toggle = label?.parentElement?.parentElement?.querySelector('button') as HTMLButtonElement | null;
      expect(toggle?.disabled).toBe(true);
      toggle?.click();
      expect(toggle?.disabled).toBe(true);
    }
  });
});
