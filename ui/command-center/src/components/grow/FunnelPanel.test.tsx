/**
 * @vitest-environment jsdom
 *
 * FunnelPanel: the consumer for the first-party funnel endpoint. Saved steps
 * re-run on mount (revisiting shows data, not an empty form), the biggest
 * drop is named, and rows the backend could not sequence (no session id) are
 * surfaced rather than hidden — a silently-filtered denominator is how
 * analytics tools lie.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import React from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({
  apiFetch: vi.fn(),
}));

vi.mock('../../lib/api', () => ({
  apiFetch,
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { FunnelPanel, type FunnelData } from './FunnelPanel';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

const funnel: FunnelData = {
  steps: [
    { label: '/pricing', sessions: 10, dropped: 0, stepRate: null, overallRate: null },
    { label: 'checkout_started', sessions: 5, dropped: 5, stepRate: 0.5, overallRate: 0.5 },
    { label: 'purchase', sessions: 4, dropped: 1, stepRate: 0.8, overallRate: 0.4 },
  ],
  conversionRate: 0.4,
  value: 0,
  biggestDropStep: 2,
  excludedSessionless: 7,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockResolvedValue(funnel);
  localStorage.clear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(ui: React.ReactElement) {
  await act(async () => root.render(ui));
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('FunnelPanel', () => {
  it('re-runs saved steps on mount and renders the funnel with its honesty line', async () => {
    localStorage.setItem('permagent-funnel-steps-p1', '/pricing,event:checkout_started,event:purchase');
    await render(<FunnelPanel projectId="p1" colors={colors} />);

    expect(apiFetch).toHaveBeenCalledWith(
      '/api/projects/p1/analytics/first_party/funnel'
      + `?steps=${encodeURIComponent('/pricing,event:checkout_started,event:purchase')}&days=30`,
    );
    // Every step row, with sessions and the step-over-step rate.
    expect(container.textContent).toContain('/pricing');
    expect(container.textContent).toContain('checkout_started');
    expect(container.textContent).toContain('purchase');
    expect(container.textContent).toContain('40% convert');
    // The worst leak is named — "what do I fix first".
    expect(container.textContent).toContain('biggest drop');
    // Sessionless rows are surfaced, not silently dropped.
    expect(container.textContent).toContain('7 matching rows carried');
  });

  it('does not fetch without saved steps; Run fetches the typed steps', async () => {
    await render(<FunnelPanel projectId="p2" colors={colors} />);
    expect(apiFetch).not.toHaveBeenCalled();

    const input = container.querySelector<HTMLInputElement>('input[aria-label="Funnel steps"]')!;
    setInputValue(input, '/a,event:b');
    const run = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Run')!;
    await act(async () => { run.click(); });

    expect(apiFetch).toHaveBeenCalledWith(
      `/api/projects/p2/analytics/first_party/funnel?steps=${encodeURIComponent('/a,event:b')}&days=30`,
    );
    // The successful run persists the steps for the next visit.
    expect(localStorage.getItem('permagent-funnel-steps-p2')).toBe('/a,event:b');
  });

  it('reports a failed compute instead of pretending the funnel is empty', async () => {
    apiFetch.mockRejectedValue(new Error('400'));
    localStorage.setItem('permagent-funnel-steps-p3', '/x');
    await render(<FunnelPanel projectId="p3" colors={colors} />);
    expect(container.textContent).toContain('Couldn’t compute the funnel');
  });
});
