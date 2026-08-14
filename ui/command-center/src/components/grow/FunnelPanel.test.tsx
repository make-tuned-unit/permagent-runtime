/**
 * @vitest-environment jsdom
 *
 * FunnelPanel: the funnel BUILDER. The step dropdowns come from what the
 * project actually recorded (never a hardcoded list), the numbers say which
 * identity they count, drop-off is reported as both a percentage and a headcount
 * lost, and time-between-steps is a median. Rows the backend could not sequence,
 * and bot rows it filtered, are surfaced rather than hidden — a silently
 * filtered denominator is how analytics tools lie.
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

import {
  FunnelPanel,
  formatDuration,
  identityLabel,
  parseSteps,
  serializeSteps,
  type FunnelData,
  type StepOptions,
} from './FunnelPanel';
import { getThemedColors } from '../../styles/tokens';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const colors = getThemedColors();

const options: StepOptions = {
  events: [
    { name: 'event_view', count: 176 },
    { name: 'cta_click', count: 17 },
  ],
  paths: [{ name: '/pricing', count: 240 }],
  periodDays: 30,
  includingBots: false,
};

const funnel: FunnelData = {
  steps: [
    { label: '/pricing', sessions: 10, dropped: 0, stepRate: null, overallRate: null, medianSecondsFromPrev: null },
    { label: 'cta_click', sessions: 5, dropped: 5, stepRate: 0.5, overallRate: 0.5, medianSecondsFromPrev: 42 },
    { label: 'event_view', sessions: 4, dropped: 1, stepRate: 0.8, overallRate: 0.4, medianSecondsFromPrev: 3720 },
  ],
  identity: 'session',
  conversionRate: 0.4,
  value: 0,
  biggestDropStep: 2,
  excludedNoIdentity: 7,
  excludedBots: 12,
};

/** Route the two endpoints the panel talks to. */
function routeApi(funnelData: FunnelData | Error = funnel) {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url.includes('step_options')) return Promise.resolve(options);
    if (funnelData instanceof Error) return Promise.reject(funnelData);
    return Promise.resolve(funnelData);
  });
}

function funnelCalls(): string[] {
  return apiFetch.mock.calls.map((c) => c[0] as string).filter((u) => u.includes('/funnel?'));
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  routeApi();
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
  // Let the step-options fetch settle too.
  await act(async () => { await Promise.resolve(); });
}

function select(label: string): HTMLSelectElement {
  return container.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`)!;
}

function button(label: string): HTMLButtonElement {
  return container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
}

function byText(text: string): HTMLButtonElement {
  return Array.from(container.querySelectorAll('button')).find((b) => b.textContent === text)!;
}

function setSelectValue(el: HTMLSelectElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLSelectElement.prototype, 'value')!.set!;
  act(() => {
    setter.call(el, value);
    el.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('funnel wire format', () => {
  it('round-trips steps and keeps the legacy bare-path form working', () => {
    expect(serializeSteps([{ type: 'path', value: '/pricing' }, { type: 'event', value: 'purchase' }]))
      .toBe('path:/pricing,event:purchase');
    expect(parseSteps('path:/pricing,event:purchase')).toEqual([
      { type: 'path', value: '/pricing' },
      { type: 'event', value: 'purchase' },
    ]);
    // What the old free-text box wrote.
    expect(parseSteps('/a,event:b')).toEqual([
      { type: 'path', value: '/a' },
      { type: 'event', value: 'b' },
    ]);
    // Empty steps never reach the wire — a trailing comma is not a step.
    expect(serializeSteps([{ type: 'event', value: '' }])).toBe('');
    expect(parseSteps('  ')).toEqual([]);
  });

  it('formats a median as a duration, and absence as absence', () => {
    expect(formatDuration(null)).toBeNull();
    expect(formatDuration(0.4)).toBe('<1s');
    expect(formatDuration(42)).toBe('42s');
    expect(formatDuration(200)).toBe('3m 20s');
    expect(formatDuration(3720)).toBe('1h 2m');
    expect(formatDuration(90_000)).toBe('1d 1h');
    expect(identityLabel('visitor')).toBe('visitors');
    expect(identityLabel('session')).toBe('sessions');
  });
});

describe('FunnelPanel builder', () => {
  it('populates the step dropdown from events the project actually recorded', async () => {
    localStorage.setItem('permagent-funnel-steps-p1', 'path:/pricing');
    await render(<FunnelPanel projectId="p1" colors={colors} />);

    expect(apiFetch).toHaveBeenCalledWith(
      '/api/projects/p1/analytics/first_party/step_options?days=30',
    );
    // Adding a step defaults to the project's most common REAL event.
    await act(async () => { byText('+ Add step').click(); });
    const second = select('Step 2');
    const optionText = Array.from(second.options).map((o) => o.textContent);
    expect(optionText).toContain('event_view (176)');
    expect(optionText).toContain('cta_click (17)');
    expect(second.value).toBe('event_view');
  });

  it('re-runs the saved funnel on mount and reports drop-off, loss and median', async () => {
    localStorage.setItem('permagent-funnel-steps-p1', 'path:/pricing,event:cta_click,event:event_view');
    await render(<FunnelPanel projectId="p1" colors={colors} />);

    expect(funnelCalls()).toEqual([
      '/api/projects/p1/analytics/first_party/funnel'
      + `?steps=${encodeURIComponent('path:/pricing,event:cta_click,event:event_view')}`
      + '&days=30&identity=session',
    ]);

    const text = container.textContent ?? '';
    expect(text).toContain('/pricing');
    expect(text).toContain('40% of sessions convert');
    // Drop-off as a rate AND as a headcount — "50%" alone hides how many people.
    expect(text).toContain('50% continued');
    expect(text).toContain('−5 lost');
    // Median time between steps, not mean.
    expect(text).toContain('median 42s');
    expect(text).toContain('median 1h 2m');
    expect(text).toContain('biggest drop');
    // The denominator names itself, and both exclusions are visible.
    expect(text).toContain('Counting SESSIONS');
    expect(text).toContain('12 bot rows excluded');
    expect(text).toContain('7 matching rows carried');
  });

  it('builds a funnel from nothing and runs it, in the order the steps were arranged', async () => {
    await render(<FunnelPanel projectId="p2" colors={colors} />);
    expect(funnelCalls()).toEqual([]); // nothing saved: no funnel is invented

    await act(async () => { byText('+ Add step').click(); });
    await act(async () => { byText('+ Add step').click(); });
    setSelectValue(select('Step 1 type'), 'path');
    setSelectValue(select('Step 1'), '/pricing');
    setSelectValue(select('Step 2'), 'cta_click');
    // Order is the funnel's meaning, so it has to be editable after the fact.
    await act(async () => { button('Move step 2 up').click(); });
    await act(async () => { byText('Run').click(); });

    expect(funnelCalls()).toEqual([
      '/api/projects/p2/analytics/first_party/funnel'
      + `?steps=${encodeURIComponent('event:cta_click,path:/pricing')}`
      + '&days=30&identity=session',
    ]);
    expect(localStorage.getItem('permagent-funnel-steps-p2')).toBe('event:cta_click,path:/pricing');
  });

  it('carries the chosen identity into the query and into what it claims to count', async () => {
    routeApi({ ...funnel, identity: 'visitor' });
    localStorage.setItem('permagent-funnel-steps-p4', 'event:cta_click');
    await render(<FunnelPanel projectId="p4" colors={colors} />);

    setSelectValue(select('Count each step by'), 'visitor');
    await act(async () => { byText('Run').click(); });

    const calls = funnelCalls();
    expect(calls[calls.length - 1]).toContain('&identity=visitor');
    expect(container.textContent).toContain('Counting VISITORS');
    expect(container.textContent).toContain('of visitors convert');
    // The exclusion line names the identity it is missing, not a generic "id".
    expect((container.textContent ?? '').replace(/\s+/g, ' ')).toContain('no visitor hash');
  });

  it('reports a failed compute instead of pretending the funnel is empty', async () => {
    routeApi(new Error('400'));
    localStorage.setItem('permagent-funnel-steps-p3', 'path:/x');
    await render(<FunnelPanel projectId="p3" colors={colors} />);
    expect(container.textContent).toContain('Couldn’t compute the funnel');
  });

  it('says so when the project has no recorded events to build from', async () => {
    apiFetch.mockReset().mockImplementation((url: string) => {
      if (url.includes('step_options')) {
        return Promise.resolve({ events: [], paths: [], periodDays: 30, includingBots: false });
      }
      return Promise.resolve(funnel);
    });
    await render(<FunnelPanel projectId="p5" colors={colors} />);
    expect(container.textContent).toContain('No events or pageviews recorded');
  });
});
