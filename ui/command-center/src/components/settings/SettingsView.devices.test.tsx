/** @vitest-environment jsdom
 *
 * Settings → Devices: "no response yet" and "fetch failed" are different
 * facts. The panel must say "Loading devices…" until the first response and
 * only use the daemon-reachable wording after a failed fetch.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  // Every other api call the panel makes on mount stays pending forever.
  api: new Proxy({ listDevices: vi.fn() } as Record<string, unknown>, {
    get: (t, k: string) => (k in t ? t[k] : () => new Promise(() => {})),
  }),
  apiFetch: vi.fn(),
  getApiBaseUrl: vi.fn(() => ''),
}));

vi.mock('./MessagingGatewaysSection', () => ({ MessagingGatewaysSection: () => null, default: () => null }));

import { api, apiFetch } from '../../lib/api';
import { DevicesPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const listDevices = vi.mocked(api.listDevices);
let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(apiFetch).mockReset().mockReturnValue(new Promise(() => {}) as never);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const status = () => container.querySelector('[data-testid="devices-status"]')?.textContent;

describe('Devices panel loading state', () => {
  it('shows Loading devices… before the first response, not the unreachable wording', async () => {
    listDevices.mockReset().mockReturnValue(new Promise(() => {}) as never);
    await act(async () => { root.render(<DevicesPanel />); });
    expect(status()).toBe('Loading devices…');
    expect(container.textContent).not.toContain('is the daemon reachable');
  });

  it('shows the daemon-reachable wording only after a failed fetch', async () => {
    listDevices.mockReset().mockRejectedValue(new Error('down'));
    await act(async () => { root.render(<DevicesPanel />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(status()).toContain('is the daemon reachable');
  });

  it('shows neither once the list arrives', async () => {
    listDevices.mockReset().mockResolvedValue([]);
    await act(async () => { root.render(<DevicesPanel />); });
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(status()).toBeUndefined();
    expect(container.textContent).toContain('No devices paired yet');
  });
});
