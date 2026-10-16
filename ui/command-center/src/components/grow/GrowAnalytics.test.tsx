/**
 * @vitest-environment jsdom
 *
 * UX_BENCHMARK_RESEARCH §8 (dagK, 2026-09-25): the four metric tiles were one
 * even grid with no hierarchy — Reach, the one tile backed by a live source,
 * sat visually equal to "Active channels" and a permanently-"—" Conversions
 * tile. This pins that Reach now renders as its own lead tile, ahead of and
 * larger than the rest, and that the rest keep rendering (nothing lost).
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({
  apiFetch,
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { computeDelta, GrowAnalytics } from './GrowAnalytics';
import { getThemedColors } from '../../styles/tokens';
import type { Project } from '../projects/types';
import type { SocialCard } from './calendarPosts';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

const project: Project = {
  id: 'p1', slug: 'p1', name: 'GrocerySaver', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [], metadataJson: {},
  createdAt: '2026-08-01T00:00:00Z', updatedAt: '2026-08-01T00:00:00Z',
  lastOpenedAt: '2026-08-01T00:00:00Z',
};

function routeAll() {
  apiFetch.mockImplementation((url: string) => {
    if (url.includes('/analytics/connection')) {
      return Promise.resolve({ connected: true, provider: 'plausible', baseUrl: 'https://x', siteId: 's', hasApiKey: true });
    }
    if (url.includes('/analytics/stats')) {
      return Promise.resolve({ connected: true, provider: 'plausible', periodDays: 30, visitors: 700, pageviews: 1023, error: null });
    }
    if (url.includes('/analytics/first_party/stats')) {
      return Promise.resolve({ receiving: false, pageviews: 0, deviceSignatures: 0, sessions: 0, byDay: [] });
    }
    if (url.includes('/publisher')) {
      return Promise.resolve({ configured: true, channels: {} });
    }
    return Promise.resolve({});
  });
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  routeAll();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function flush() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

it('clears the previous project metrics and distinguishes unavailable sources from empty sources', async () => {
  const pending: Array<(reason: Error) => void> = [];
  apiFetch.mockImplementation((url: string) => {
    if (url.includes('/p2/') && (url.endsWith('/analytics/connection') || url.endsWith('/analytics/first_party/stats') || url.endsWith('/publisher'))) {
      return new Promise((_resolve, reject) => pending.push(reject));
    }
    if (url.endsWith('/analytics/connection')) return Promise.resolve({ connected: false });
    if (url.endsWith('/analytics/first_party/stats')) return Promise.resolve({ receiving: true, pageviews: 987, deviceSignatures: 100, sessions: 150, byDay: [], period: { days: 30 } });
    if (url.includes('/analytics/first_party/step_options')) return Promise.resolve({ events: [], paths: [], periodDays: 30, includingBots: false });
    if (url.endsWith('/publisher')) return Promise.resolve({ configured: true, channels: { instagram: { integrationId: 'fixture-channel' } } });
    return Promise.resolve({});
  });
  await act(async () => root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />));
  await flush();
  expect(container.querySelector('[data-testid="metric-tile-lead"]')?.textContent).toContain('987');
  await act(async () => root.render(<GrowAnalytics project={{ ...project, id: 'p2', name: 'New project' }} posts={[]} colors={colors} />));
  expect(container.querySelector('[data-testid="metric-tile-lead"]')?.textContent).not.toContain('987');
  expect(container.textContent).toContain('Checking analytics sources');
  expect(container.textContent).toContain('Connection status unknown');
  expect(container.textContent).not.toContain('No source is receiving');
  await act(async () => pending.forEach(reject => reject(new Error('offline'))));
  await flush();
  expect(container.textContent).toContain('Analytics source status is unavailable');
  expect(container.textContent).not.toContain('No source is receiving');
  expect(Array.from(container.querySelectorAll('button')).some(button => button.textContent === 'Retry analytics')).toBe(true);
});

it('renders Reach as its own lead tile, ahead of the setup/config tiles', async () => {
  const posts: SocialCard[] = [];
  await act(async () => {
    root.render(<GrowAnalytics project={project} posts={posts} colors={colors} />);
  });
  await flush();

  const lead = container.querySelector('[data-testid="metric-tile-lead"]');
  expect(lead, 'lead tile').toBeTruthy();
  expect(lead!.textContent).toContain('PAGEVIEWS (30D)');
  expect(lead!.textContent).toContain('1,023');

  const rest = Array.from(container.querySelectorAll('[data-testid="metric-tile"]'));
  const labels = rest.map((el) => el.textContent);
  expect(labels.some((t) => t?.includes('POSTS PUBLISHED'))).toBe(true);
  expect(labels.some((t) => t?.includes('ACTIVE CHANNELS'))).toBe(true);
  expect(labels.some((t) => t?.includes('CONVERSIONS'))).toBe(true);
  // Reach must not also be duplicated among the smaller tiles.
  expect(labels.some((t) => t?.includes('REACH'))).toBe(false);

  // The lead tile precedes the grid of the rest in document order.
  const grid = rest[0];
  expect(lead!.compareDocumentPosition(grid) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
});

// ── Period-over-period delta (dagK grow-deltas, 2026-09-26) ─────────────────
//
// `computeDelta` is pure, so the arithmetic behind the chip is pinned
// directly; the rendering tests below cover the two live sources (the
// third-party connector and the first-party self-hosted collector) and the
// "no prior period" honesty rule.

describe('computeDelta', () => {
  it('reports unknown, never a fabricated flat, when there is no prior figure', () => {
    expect(computeDelta(100, null)).toEqual({ direction: 'unknown', pct: null });
    expect(computeDelta(100, undefined)).toEqual({ direction: 'unknown', pct: null });
    expect(computeDelta(null, 100)).toEqual({ direction: 'unknown', pct: null });
  });

  it('computes an up or down percentage against a positive previous figure', () => {
    expect(computeDelta(120, 100)).toEqual({ direction: 'up', pct: 20 });
    expect(computeDelta(80, 100)).toEqual({ direction: 'down', pct: -20 });
  });

  it('reports flat with 0% when the two figures are exactly equal', () => {
    expect(computeDelta(50, 50)).toEqual({ direction: 'flat', pct: 0 });
    expect(computeDelta(0, 0)).toEqual({ direction: 'flat', pct: 0 });
  });

  it('reports up with no percentage when the previous period was genuinely zero', () => {
    // 0 -> N has no percentage form (division by zero); the direction is
    // still knowable (anything beats zero), the percentage is not.
    expect(computeDelta(10, 0)).toEqual({ direction: 'up', pct: null });
  });
});

describe('GrowAnalytics — Reach delta chip', () => {
  it('shows an up delta against the connector\'s previous period', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/analytics/connection')) {
        return Promise.resolve({ connected: true, provider: 'plausible', baseUrl: 'https://x', siteId: 's', hasApiKey: true });
      }
      if (url.includes('/analytics/stats')) {
        return Promise.resolve({
          connected: true, provider: 'plausible', periodDays: 30, visitors: 700, pageviews: 1200, error: null,
          period: { start: '2026-08-01', end: '2026-08-30', days: 30 },
          previousPeriod: { visitors: 600, pageviews: 1000 },
        });
      }
      if (url.includes('/analytics/first_party/stats')) {
        return Promise.resolve({ receiving: false, pageviews: 0, deviceSignatures: 0, sessions: 0, byDay: [] });
      }
      if (url.includes('/publisher')) return Promise.resolve({ configured: true, channels: {} });
      return Promise.resolve({});
    });
    await act(async () => {
      root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />);
    });
    await flush();

    const chip = container.querySelector('[data-testid="reach-delta"]');
    expect(chip, 'delta chip').toBeTruthy();
    expect(chip!.textContent).toContain('▲');
    expect(chip!.textContent).toContain('20%');
  });

  it('shows a down delta when the current period is lower than the previous one', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/analytics/connection')) {
        return Promise.resolve({ connected: true, provider: 'plausible', baseUrl: 'https://x', siteId: 's', hasApiKey: true });
      }
      if (url.includes('/analytics/stats')) {
        return Promise.resolve({
          connected: true, provider: 'plausible', periodDays: 30, visitors: 400, pageviews: 800, error: null,
          period: { start: '2026-08-01', end: '2026-08-30', days: 30 },
          previousPeriod: { visitors: 600, pageviews: 1000 },
        });
      }
      if (url.includes('/analytics/first_party/stats')) {
        return Promise.resolve({ receiving: false, pageviews: 0, deviceSignatures: 0, sessions: 0, byDay: [] });
      }
      if (url.includes('/publisher')) return Promise.resolve({ configured: true, channels: {} });
      return Promise.resolve({});
    });
    await act(async () => {
      root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />);
    });
    await flush();

    const chip = container.querySelector('[data-testid="reach-delta"]');
    expect(chip!.textContent).toContain('▼');
    expect(chip!.textContent).toContain('20%');
  });

  it('shows "no prior period" — never a fabricated 0% — when previousPeriod is null', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/analytics/connection')) {
        return Promise.resolve({ connected: true, provider: 'plausible', baseUrl: 'https://x', siteId: 's', hasApiKey: true });
      }
      if (url.includes('/analytics/stats')) {
        return Promise.resolve({
          connected: true, provider: 'plausible', periodDays: 30, visitors: 400, pageviews: 800, error: null,
          period: { start: '2026-08-01', end: '2026-08-30', days: 30 },
          previousPeriod: null,
        });
      }
      if (url.includes('/analytics/first_party/stats')) {
        return Promise.resolve({ receiving: false, pageviews: 0, deviceSignatures: 0, sessions: 0, byDay: [] });
      }
      if (url.includes('/publisher')) return Promise.resolve({ configured: true, channels: {} });
      return Promise.resolve({});
    });
    await act(async () => {
      root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />);
    });
    await flush();

    const chip = container.querySelector('[data-testid="reach-delta"]');
    expect(chip!.textContent).toContain('no prior period');
  });

  it('reads the delta from the first-party collector once it is live, not the connector', async () => {
    apiFetch.mockImplementation((url: string) => {
      if (url.includes('/analytics/connection')) {
        return Promise.resolve({ connected: false, provider: null, baseUrl: null, siteId: null, hasApiKey: false });
      }
      if (url.includes('/analytics/first_party/stats')) {
        return Promise.resolve({
          receiving: true, pageviews: 300, deviceSignatures: 120, sessions: 50, byDay: [],
          period: { start: '2026-09-01T00:00:00Z', end: '2026-09-26T00:00:00Z', days: 30 },
          previousPeriod: { pageviews: 200, deviceSignatures: 90 },
        });
      }
      // `fpLive` mounts the FunnelPanel, which fetches its step options eagerly.
      if (url.includes('/analytics/first_party/step_options')) {
        return Promise.resolve({ events: [], paths: [], periodDays: 30, includingBots: false });
      }
      if (url.includes('/publisher')) return Promise.resolve({ configured: true, channels: {} });
      return Promise.resolve({});
    });
    await act(async () => {
      root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />);
    });
    await flush();

    const lead = container.querySelector('[data-testid="metric-tile-lead"]');
    expect(lead!.textContent).toContain('300');
    const chip = container.querySelector('[data-testid="reach-delta"]');
    // (300 - 200) / 200 = 50%.
    expect(chip!.textContent).toContain('▲');
    expect(chip!.textContent).toContain('50%');
  });
});

it('leads with measures and discloses installation and coverage without losing them', async () => {
  await act(async () => root.render(<GrowAnalytics project={project} posts={[]} colors={colors} />));
  await flush();
  const details = Array.from(container.querySelectorAll('details'));
  const source = details.find(el => el.querySelector('summary')?.textContent === 'Source data and installation')!;
  expect(source.open).toBe(false);
  const lead = container.querySelector('[data-testid="metric-tile-lead"]')!;
  expect(lead.compareDocumentPosition(source) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(container.textContent).toContain('not conversion rates');
  expect(apiFetch.mock.calls.every(([, options]) => !options?.method || options.method === 'GET')).toBe(true);
});
