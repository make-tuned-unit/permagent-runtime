/**
 * @vitest-environment jsdom
 *
 * The Market card's three honesty rules, each pinned by a test:
 *
 *  1. A forecast never appears without the method that produced it.
 *  2. A series that cannot be forecast renders the REASON, not an empty chart.
 *  3. "Nothing is bound" is rendered as nothing being bound — never as a flat
 *     market, and never as a forecast of zero.
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { MarketPanel } from './MarketPanel';
import type { Project } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const project = { id: 'project/1', name: 'Acme' } as Project;

const baseRow = {
  seriesId: 's1',
  sourceKind: 'npm',
  sourceLabel: 'npm downloads',
  subject: 'langchain',
  subjectGroup: 'langchain',
  cadence: 'daily' as const,
  label: 'langchain — npm downloads',
  status: 'active' as const,
  points: 200,
  spanDays: 199,
  snapshotOnly: false,
  officialSource: true,
  lastError: null,
  history: [100, 105, 102, 110, 108, 115],
  forecast: null,
  refusal: null,
  direction: null,
};

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

it('never shows a number without the method that produced it', async () => {
  apiFetch.mockResolvedValue({
    noSeriesBound: false,
    generatedAt: '2026-08-24T00:00:00.000Z',
    rows: [{
      ...baseRow,
      verdict: 'forecastable',
      direction: 'up 9% over the next 7 steps',
      forecast: {
        point: [120, 121, 122, 123, 124, 125, 126],
        p10: [110, 110, 110, 110, 110, 110, 110],
        p90: [130, 130, 130, 130, 130, 130, 130],
        method: 'seasonal_naive',
        methodLabel: 'seasonal naive — last week repeated, not a model',
        maseVsBaseline: 1.0,
        folds: 8,
        foldWins: 0,
        selection: 'ETS did not clear the gate',
      },
    }],
  });
  await act(async () => root.render(<MarketPanel project={project} />));
  const text = container.textContent ?? '';
  expect(text).toContain('up 9% over the next 7 steps');
  // The label rides with the number, always.
  expect(text).toContain('method:');
  expect(text).toContain('not a model');
  expect(text).toContain('MASE');
});

it('renders the reason a short series cannot be forecast, not an empty chart', async () => {
  apiFetch.mockResolvedValue({
    noSeriesBound: false,
    generatedAt: '2026-08-24T00:00:00.000Z',
    rows: [{
      ...baseRow,
      points: 42,
      verdict: 'insufficient_history',
      refusal: { reason: 'insufficient_history', points: 42, needed: 180 },
    }],
  });
  await act(async () => root.render(<MarketPanel project={project} />));
  const text = container.textContent ?? '';
  expect(text).toContain('42 of 180');
  expect(text).toContain('too short to forecast');
  // No method label, because no method ran.
  expect(text).not.toContain('method:');
});

it('says a stale collector is stale rather than showing its last numbers as current', async () => {
  apiFetch.mockResolvedValue({
    noSeriesBound: false,
    generatedAt: '2026-08-24T00:00:00.000Z',
    rows: [{
      ...baseRow,
      verdict: 'collector_stale',
      lastError: 'source unreachable: HTTP 503',
      refusal: { reason: 'collector_stale', lastCollectedAt: '2026-07-01T00:00:00.000Z' },
    }],
  });
  await act(async () => root.render(<MarketPanel project={project} />));
  const text = container.textContent ?? '';
  // Said in the user's terms rather than the daemon's: "collector_stale" is a
  // refusal reason, and "Collector stale" was that word rendered straight
  // through. What the reader needs is that the data is old and why.
  expect(text).toContain('No recent data');
  expect(text).toContain('last ran');
  expect(text).toContain('collector error');
  expect(text).not.toContain('method:');
});

it('distinguishes nothing bound from a flat market', async () => {
  apiFetch.mockResolvedValue({
    noSeriesBound: true,
    generatedAt: '2026-08-24T00:00:00.000Z',
    rows: [],
  });
  await act(async () => root.render(<MarketPanel project={project} />));
  const text = container.textContent ?? '';
  expect(text).toContain('No market series bound');
  expect(text).toContain('is a forecast of zero');
});

it('labels a snapshot-only source and an unofficial one where the user sees them', async () => {
  apiFetch.mockResolvedValue({
    noSeriesBound: false,
    generatedAt: '2026-08-24T00:00:00.000Z',
    rows: [{
      ...baseRow,
      seriesId: 's2',
      sourceKind: 'github_repo',
      sourceLabel: 'GitHub stars',
      subject: 'ollama/ollama',
      points: 3,
      snapshotOnly: true,
      officialSource: false,
      verdict: 'insufficient_history',
      refusal: { reason: 'insufficient_history', points: 3, needed: 180 },
    }],
  });
  await act(async () => root.render(<MarketPanel project={project} />));
  const text = container.textContent ?? '';
  expect(text).toContain('snapshot-only');
  expect(text).toContain('unofficial source');
});
