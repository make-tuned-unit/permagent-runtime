/**
 * @vitest-environment jsdom
 *
 * `<JobProgress>` — the five phases a screenshot would show, each reachable
 * from props alone. The assertions that matter are the honesty ones: a job
 * with no reported size must not draw a determinate bar, a failure must print
 * the backend's own words, and a user's Stop must not be dressed as a crash.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

import { JobProgress } from './JobProgress';
import type { LongRunningJob, JobPhase, JobReading } from '../../hooks/useLongRunningJob';
import { percentOf } from '../../hooks/useLongRunningJob';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

function job(over: Partial<LongRunningJob<unknown>> & { phase: JobPhase; reading?: JobReading }): LongRunningJob<unknown> {
  const reading = over.reading ?? {};
  return {
    phase: over.phase,
    running: over.phase === 'starting' || over.phase === 'running',
    reading,
    percent: over.percent !== undefined ? over.percent : percentOf(reading),
    error: over.error ?? null,
    result: over.result ?? null,
    summary: over.summary ?? null,
    startedAt: over.startedAt ?? null,
    finishedAt: over.finishedAt ?? null,
    start: over.start ?? (async () => {}),
    abort: over.abort ?? (() => {}),
    reset: over.reset ?? (() => {}),
  };
}

function render(node: React.ReactElement) {
  act(() => root.render(node));
}

function testid(id: string): HTMLElement | null {
  return container.querySelector(`[data-testid="${id}"]`);
}

it('draws nothing at all while idle', () => {
  render(<JobProgress job={job({ phase: 'idle' })} label="Scan the universe" />);
  expect(testid('job-progress')).toBeNull();
});

it('names the phase on the root so a screenshot is self-describing', () => {
  render(<JobProgress job={job({ phase: 'running' })} label="Scanning" />);
  expect(testid('job-progress')?.getAttribute('data-phase')).toBe('running');
});

it('with a reported size: determinate bar and a mono percentage', () => {
  render(
    <JobProgress
      job={job({ phase: 'running', reading: { status: 'Layer 3 of 8', completed: 3, total: 8 } })}
      label="Downloading"
    />,
  );
  const bar = testid('job-progress-bar');
  expect(bar?.getAttribute('data-determinate')).toBe('true');
  expect((bar as HTMLElement).style.width).toBe('38%');
  expect(testid('job-progress-percent')?.textContent).toBe('38%');
  expect(container.textContent).toContain('Layer 3 of 8');
});

it('with no reported size: indeterminate, and no invented percentage', () => {
  render(<JobProgress job={job({ phase: 'running', reading: { status: 'Fetching tickers' } })} label="Scanning" />);
  expect(testid('job-progress-bar')?.getAttribute('data-determinate')).toBe('false');
  expect(testid('job-progress-percent')).toBeNull();
  expect(container.textContent).toContain('Fetching tickers');
});

it('offers Stop only when the job can actually be stopped', () => {
  const abort = vi.fn();
  render(<JobProgress job={job({ phase: 'running', abort })} label="Scanning" onStop={abort} />);
  const stop = testid('job-progress-stop') as HTMLButtonElement;
  expect(stop).not.toBeNull();
  act(() => { stop.click(); });
  expect(abort).toHaveBeenCalled();

  render(<JobProgress job={job({ phase: 'running' })} label="Scanning" />);
  expect(testid('job-progress-stop')).toBeNull();
});

it('success names what completed rather than just vanishing', () => {
  render(
    <JobProgress
      job={job({ phase: 'succeeded', summary: 'Scan complete — 4 findings' })}
      label="Scanning"
    />,
  );
  expect(testid('job-progress')?.getAttribute('data-phase')).toBe('succeeded');
  expect(container.textContent).toContain('Scan complete — 4 findings');
});

it("failure prints the backend's own message and offers a retry", () => {
  const start = vi.fn(async () => {});
  render(
    <JobProgress
      job={job({ phase: 'failed', error: 'scanner refused the connection', start })}
      label="Scanning"
    />,
  );
  expect(container.textContent).toContain('scanner refused the connection');
  const retry = testid('job-progress-retry') as HTMLButtonElement;
  act(() => { retry.click(); });
  expect(start).toHaveBeenCalled();
});

it('a stopped job says stopped — not failed, and with no error text', () => {
  render(<JobProgress job={job({ phase: 'stopped' })} label="Scanning" />);
  const root_ = testid('job-progress');
  expect(root_?.getAttribute('data-phase')).toBe('stopped');
  expect(root_?.textContent?.toLowerCase()).toContain('stopped');
  expect(root_?.textContent?.toLowerCase()).not.toContain('failed');
});

it("shows the backend's own stage name when it gives one", () => {
  render(
    <JobProgress
      job={job({ phase: 'running', reading: { stage: 'loading_model', status: 'mmap 40%' } })}
      label="Starting Qwen"
    />,
  );
  expect(container.textContent).toContain('loading_model');
});
