/**
 * @vitest-environment jsdom
 *
 * UX_BENCHMARK_RESEARCH §8 (dagK, 2026-09-25): the four metric tiles were one
 * even grid with no hierarchy — Reach, the one tile backed by a live source,
 * sat visually equal to "Active channels" and a permanently-"—" Conversions
 * tile. This pins that Reach now renders as its own lead tile, ahead of and
 * larger than the rest, and that the rest keep rendering (nothing lost).
 */

import { expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({
  apiFetch,
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { GrowAnalytics } from './GrowAnalytics';
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

it('renders Reach as its own lead tile, ahead of the setup/config tiles', async () => {
  const posts: SocialCard[] = [];
  await act(async () => {
    root.render(<GrowAnalytics project={project} posts={posts} colors={colors} />);
  });
  await flush();

  const lead = container.querySelector('[data-testid="metric-tile-lead"]');
  expect(lead, 'lead tile').toBeTruthy();
  expect(lead!.textContent).toContain('REACH (30D)');
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
