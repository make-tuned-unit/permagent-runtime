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

  it('ignores an older poll response that resolves after a newer one', async () => {
    vi.useFakeTimers();
    let resolveOld!: (value: unknown) => void;
    let resolveNew!: (value: unknown) => void;
    apiFetch
      .mockReturnValueOnce(new Promise(resolve => { resolveOld = resolve; }))
      .mockReturnValueOnce(new Promise(resolve => { resolveNew = resolve; }));

    await act(async () => {
      root.render(<ManifestCard manifest={{ ...statManifest, refreshSeconds: 1 }} />);
    });
    await act(async () => { vi.advanceTimersByTime(1_000); });
    await act(async () => { resolveNew({ cells: [{ label: 'Generation', value: 'new' }] }); });
    expect(container.textContent).toContain('new');

    await act(async () => { resolveOld({ cells: [{ label: 'Generation', value: 'old' }] }); });
    expect(container.textContent).toContain('new');
    expect(container.textContent).not.toContain('old');
    vi.useRealTimers();
  });
});

describe('ManifestCard — compact tile grouping', () => {
  const compact: CardManifest = { ...weatherManifest, layout: 'compact', configure: undefined };

  it('draws grouped forecast cells with their day labels, not as anonymous values', async () => {
    // The inline treatment renders icon + value only, which is right for a
    // humidity reading and useless for four days of weather.
    apiFetch.mockResolvedValueOnce({
      cells: [
        { label: 'Halifax', value: '26° Clear', accent: true, icon: 'sun' },
        { label: 'Humidity', value: '77%', icon: 'droplet' },
        { label: 'Sat', value: '24° / 17°', sub: '80% rain', icon: 'rain', group: 'forecast' },
        { label: 'Sun', value: '21° / 15°', icon: 'cloud', group: 'forecast' },
      ],
    });
    await act(async () => { root.render(<ManifestCard manifest={compact} />); });
    await flush();

    const strip = container.querySelector('[aria-label="Forecast"]');
    expect(strip).toBeTruthy();
    expect(strip!.textContent).toContain('Sat');
    expect(strip!.textContent).toContain('24° / 17°');
    expect(strip!.textContent).toContain('80% rain');
    expect(strip!.querySelectorAll('[role="listitem"]')).toHaveLength(2);

    // The ungrouped supporting cell stays out of the strip.
    expect(strip!.textContent).not.toContain('77%');
    expect(container.textContent).toContain('77%');
  });

  it('has no forecast strip when nothing is grouped', async () => {
    apiFetch.mockResolvedValueOnce({
      cells: [
        { label: 'CPU load', value: '1231%', accent: true, icon: 'cpu' },
        { label: 'Disk', value: '31.4 GB free · 93% used', icon: 'disk' },
      ],
    });
    await act(async () => { root.render(<ManifestCard manifest={compact} />); });
    await flush();

    expect(container.querySelector('[aria-label="Forecast"]')).toBeNull();
    expect(container.textContent).toContain('31.4 GB free · 93% used');
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

describe('a card that fetches once and never polls', () => {
  it('dates its reading', async () => {
    // `refreshSeconds: 0` (or absent) means one fetch on mount, forever. Those
    // figures sit beside cards refreshing every thirty seconds, in the same
    // type, with nothing distinguishing them — so a number frozen since the tab
    // was opened reads exactly like one confirmed a moment ago.
    apiFetch.mockResolvedValue({ cells: [{ label: 'Uptime', value: '3d' }] });
    await act(async () => root.render(<ManifestCard manifest={statManifest} />));
    await flush();
    const asOf = container.querySelector('[data-testid="manifest-card-as-of"]');
    expect(asOf).not.toBeNull();
    expect(asOf!.textContent).toContain('not refreshing');
  });

  it('leaves a polling card alone', async () => {
    apiFetch.mockResolvedValue({ cells: [{ label: 'Uptime', value: '3d' }] });
    const polling: CardManifest = { ...statManifest, refreshSeconds: 30 };
    await act(async () => root.render(<ManifestCard manifest={polling} />));
    await flush();
    // A card that is current by construction does not need to say so, and a
    // timestamp on every card would be noise that teaches nobody anything.
    expect(container.querySelector('[data-testid="manifest-card-as-of"]')).toBeNull();
  });
});
