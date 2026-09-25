/** @vitest-environment jsdom
 *
 * A6: one line at the top of Automate saying either "everything is running"
 * or which jobs need attention. R2: the strip must not render at all before
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

it('says "everything is running" in the ok state', () => {
  const summary = summarizeHealth({ jobs: [{ id: 'a', label: 'A', cadence: 'daily', outcome: 'ok' }] }, []);
  act(() => root.render(<JobHealthStrip summary={summary} />));
  const strip = container.querySelector('[data-testid="job-health-strip"]')!;
  expect(strip.getAttribute('data-state')).toBe('ok');
  expect(strip.textContent).toContain('Everything is running');
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
