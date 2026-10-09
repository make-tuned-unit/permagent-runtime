/** @vitest-environment jsdom
 *
 * A6: one line at the top of Automate reporting paused schedules, known
 * problems, or digest availability. R2: the strip must not render before
 * the first fetch settles (its own "loading" state is its absence, not a
 * false green), and a resolved-but-unknown digest must say so, not go quiet.
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

import { JobHealthStrip } from './JobHealthStrip';
import { summarizeHealth } from './jobHealthModel';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

it('renders nothing for a null summary — the loading state is its own absence', () => {
  act(() => root.render(<JobHealthStrip summary={null} />));
  expect(container.querySelector('[data-testid="job-health-strip"]')).toBeNull();
});

it('reports no known problems without claiming a job ran in the ok state', () => {
  const summary = summarizeHealth({ jobs: [{ id: 'a', label: 'A', cadence: 'daily', outcome: 'ok' }] }, []);
  act(() => root.render(<JobHealthStrip summary={summary} />));
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('ok');
  expect(strip.textContent).toContain('No current job-health problems reported');
});

it('renders all-paused schedules as neutral, including review count', () => {
  const summary = summarizeHealth({ jobs: [{ id: 'audit', label: 'Audit', cadence: 'built-in', outcome: 'ok' }] }, [
    { id: 'starter', paused: true, currently_running: false, requires_approval: true },
  ]);
  act(() => root.render(<JobHealthStrip summary={summary} />));
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('paused');
  expect(strip.textContent).toContain('1 schedule paused — 1 needs review before enabling');
});

it('says job health is unknown rather than claiming green when the fetch failed', () => {
  const summary = summarizeHealth({ error: 'the daemon did not answer' }, []);
  act(() => root.render(<JobHealthStrip summary={summary} />));
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('unknown');
  expect(strip.textContent).toContain('unknown');
});

it('lists each problem, opening one that has a page here and not one that does not', () => {
  const summary = summarizeHealth({
    jobs: [
      { id: 'flight-monitor', label: 'Flight monitor', cadence: 'hourly', outcome: 'failed', failure_streak: 2 },
      { id: 'the-guard', label: 'The Guard', cadence: 'builtin', outcome: 'missed' },
    ],
  }, []);
  const onOpen = vi.fn();
  act(() => root.render(
    <JobHealthStrip summary={summary} onOpen={onOpen} canOpen={id => id === 'flight-monitor'} />,
  ));
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.textContent).toContain('Flight monitor');
  expect(strip.textContent).toContain('The Guard');

  const problems = strip.querySelectorAll('[data-testid="job-health-problem"]');
  expect(problems).toHaveLength(2);
  const openable = [...problems].find(p => p.tagName === 'BUTTON')!;
  expect(openable.textContent).toContain('Flight monitor');
  act(() => { openable.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(onOpen).toHaveBeenCalledWith('flight-monitor');

  // The Guard has no detail page on this screen — a plain label, not a button.
  const notOpenable = [...problems].find(p => p.textContent?.includes('The Guard'))!;
  expect(notOpenable.tagName).not.toBe('BUTTON');
});
