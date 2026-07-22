/**
 * @vitest-environment jsdom
 *
 * ManifestCard (#182 renderer / #181 cards) — self-fetch + layout + configure.
 *
 * Pins that the generic renderer fetches its manifest's dataEndpoint, draws the
 * declared layout from the normalized CardData, and that a `configured: false`
 * response with a configure block shows the inline setup flow which PUTs
 * `{ query }` and refetches.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const apiFetch = vi.fn();
vi.mock('../../../lib/api', () => ({ apiFetch: (...a: unknown[]) => apiFetch(...a) }));

import { ManifestCard } from './ManifestCard';
import type { CardManifest } from './registry';

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
  // Let the fetch promise + state updates settle.
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

const statManifest: CardManifest = {
  type: 'system_stats',
  name: 'System',
  description: 'stats',
  defaultSize: { w: 5, h: 4 },
  layout: 'stat-grid',
  dataEndpoint: '/api/dashboard/system-stats',
  source: 'built-in',
};

const weatherManifest: CardManifest = {
  type: 'weather',
  name: 'Weather',
  description: 'weather',
  defaultSize: { w: 5, h: 4 },
  layout: 'stat-grid',
  dataEndpoint: '/api/dashboard/weather',
  source: 'built-in',
  configure: {
    endpoint: '/api/dashboard/weather/location',
    label: 'Set location',
    placeholder: 'City',
  },
};

describe('ManifestCard — data + layout', () => {
  it('fetches the manifest endpoint and renders stat-grid cells', async () => {
    apiFetch.mockResolvedValueOnce({
      cells: [
        { label: 'CPU load', value: '42%', accent: true },
        { label: 'Memory', value: '8.5 / 16 GB' },
      ],
    });
    await act(async () => { root.render(<ManifestCard manifest={statManifest} />); });
    await flush();

    expect(apiFetch).toHaveBeenCalledWith('/api/dashboard/system-stats');
    expect(container.textContent).toContain('CPU load');
    expect(container.textContent).toContain('42%');
    expect(container.textContent).toContain('8.5 / 16 GB');
  });

  it('shows the endpoint note when there are no cells', async () => {
    apiFetch.mockResolvedValueOnce({ cells: [], note: 'No events today' });
    await act(async () => { root.render(<ManifestCard manifest={{ ...statManifest, layout: 'list' }} />); });
    await flush();
    expect(container.textContent).toContain('No events today');
  });
});

describe('ManifestCard — configure flow', () => {
  it('renders the setup affordance when the endpoint reports unconfigured, then PUTs and refetches', async () => {
    // 1st fetch: not configured. PUT resolves. 2nd fetch: configured with data.
    apiFetch
      .mockResolvedValueOnce({ configured: false, note: 'Set your location' })
      .mockResolvedValueOnce({}) // the PUT
      .mockResolvedValueOnce({ cells: [{ label: 'San Francisco', value: '18° Clear' }] });

    await act(async () => { root.render(<ManifestCard manifest={weatherManifest} />); });
    await flush();

    expect(container.textContent).toContain('Set your location');
    const setBtn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Set location');
    expect(setBtn).toBeTruthy();

    // Open the input.
    act(() => setBtn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    const input = container.querySelector('input') as HTMLInputElement;
    expect(input).toBeTruthy();

    // Type a city and submit.
    act(() => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
      setter.call(input, 'San Francisco');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const submit = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Set');
    expect(submit).toBeTruthy();
    await act(async () => {
      submit!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      await Promise.resolve();
    });
    await flush();

    expect(apiFetch).toHaveBeenCalledWith('/api/dashboard/weather/location', expect.objectContaining({ method: 'PUT' }));
    expect(container.textContent).toContain('San Francisco');
    expect(container.textContent).toContain('18° Clear');
  });
});
