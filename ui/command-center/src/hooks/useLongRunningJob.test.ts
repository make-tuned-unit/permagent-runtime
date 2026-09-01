/**
 * @vitest-environment jsdom
 *
 * The long-running-job phase machine. Every assertion here exists because the
 * corresponding lie is one a UI can tell: a job that finished but still looks
 * busy, a failure that lands back on idle and reads as "nothing happened", a
 * user's own Stop reported as a crash, or a percentage invented for work whose
 * size the backend never gave.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { createElement } from 'react';

import {
  useLongRunningJob,
  pollingRunner,
  streamingRunner,
  type LongRunningJob,
} from './useLongRunningJob';

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

/** Mounts the hook and hands back a live handle to its latest return value. */
function mount<T>(options: Parameters<typeof useLongRunningJob<T>>[0]) {
  const box: { current: LongRunningJob<T> } = { current: null as never };
  function Probe() {
    box.current = useLongRunningJob<T>(options);
    return null;
  }
  act(() => root.render(createElement(Probe)));
  return box;
}

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

it('starts idle and reports no percentage before anything has run', () => {
  const job = mount<void>({ run: async () => {} });
  expect(job.current.phase).toBe('idle');
  expect(job.current.running).toBe(false);
  expect(job.current.percent).toBeNull();
  expect(job.current.error).toBeNull();
});

it('moves idle → running → succeeded and keeps the summary', async () => {
  const d = deferred<number>();
  const job = mount<number>({
    run: async ({ report }) => { report({ status: 'Scanning...' }); return d.promise; },
    summarize: (n) => `Scan complete — ${n} findings`,
  });

  await act(async () => { void job.current.start(); });
  expect(job.current.phase).toBe('running');
  expect(job.current.running).toBe(true);
  expect(job.current.reading.status).toBe('Scanning...');

  await act(async () => { d.resolve(4); await d.promise; });
  expect(job.current.phase).toBe('succeeded');
  expect(job.current.running).toBe(false);
  expect(job.current.summary).toBe('Scan complete — 4 findings');
  expect(job.current.result).toBe(4);
  expect(job.current.finishedAt).not.toBeNull();
});

it('a failure is its own terminal phase carrying the real message', async () => {
  const d = deferred<void>();
  const job = mount<void>({ run: () => d.promise });

  await act(async () => { void job.current.start(); });
  await act(async () => {
    d.reject(new Error('scanner refused the connection'));
    await d.promise.catch(() => {});
  });

  expect(job.current.phase).toBe('failed');
  // Never idle: an idle-looking control after a failure is the exact lie
  // MomentHardware's comment forbids.
  expect(job.current.phase).not.toBe('idle');
  expect(job.current.error).toBe('scanner refused the connection');
});

it("a user's own Stop is 'stopped', never 'failed'", async () => {
  const job = mount<void>({
    run: ({ signal }) => new Promise<void>((_res, rej) => {
      signal.addEventListener('abort', () => {
        const e = new Error('aborted');
        e.name = 'AbortError';
        rej(e);
      });
    }),
  });

  await act(async () => { void job.current.start(); });
  await act(async () => { job.current.abort(); await Promise.resolve(); });

  expect(job.current.phase).toBe('stopped');
  expect(job.current.error).toBeNull();
});

it('derives a percentage only when the backend gave a size', async () => {
  const d = deferred<void>();
  let push: (r: { completed?: number; total?: number }) => void = () => {};
  const job = mount<void>({
    run: async ({ report }) => { push = report; return d.promise; },
  });

  await act(async () => { void job.current.start(); });
  expect(job.current.percent).toBeNull();

  await act(async () => { push({ completed: 3, total: 8 }); });
  expect(job.current.percent).toBe(38);

  // A total of 0 is not 100% and not 0% — it is "no size given".
  await act(async () => { push({ completed: 0, total: 0 }); });
  expect(job.current.percent).toBeNull();
});

it('reset returns to idle and clears the last outcome', async () => {
  const job = mount<void>({ run: async () => { throw new Error('nope'); } });
  await act(async () => { await job.current.start(); });
  expect(job.current.phase).toBe('failed');

  act(() => job.current.reset());
  expect(job.current.phase).toBe('idle');
  expect(job.current.error).toBeNull();
});

it('refuses a second concurrent start', async () => {
  const d = deferred<void>();
  const run = vi.fn(() => d.promise);
  const job = mount<void>({ run });

  await act(async () => { void job.current.start(); });
  await act(async () => { void job.current.start(); });
  expect(run).toHaveBeenCalledTimes(1);

  await act(async () => { d.resolve(); await d.promise; });
});

it('pollingRunner reports each tick and resolves on the terminal one', async () => {
  const ticks = [
    { done: false, reading: { stage: 'queued', status: 'Queued' } },
    { done: false, reading: { stage: 'scanning', status: 'Scanning 12/40', completed: 12, total: 40 } },
    { done: true, result: 7, reading: { status: 'Done' } },
  ];
  let i = 0;
  const begin = vi.fn(async () => {});
  const job = mount<number>({
    run: pollingRunner<number>({ begin, poll: async () => ticks[i++]!, intervalMs: 1 }),
    summarize: (n) => `${n} findings`,
  });

  await act(async () => { await job.current.start(); });
  expect(begin).toHaveBeenCalledTimes(1);
  expect(job.current.phase).toBe('succeeded');
  expect(job.current.result).toBe(7);
});

it('pollingRunner turns a backend-reported error into a failure', async () => {
  const job = mount<void>({
    run: pollingRunner<void>({
      begin: async () => {},
      poll: async () => ({ done: true, error: 'the sweep died' }),
      intervalMs: 1,
    }),
  });
  await act(async () => { await job.current.start(); });
  expect(job.current.phase).toBe('failed');
  expect(job.current.error).toBe('the sweep died');
});

it('streamingRunner maps stream frames onto readings and honours abort', async () => {
  const d = deferred<void>();
  const abort = vi.fn(() => {
    const e = new Error('aborted');
    e.name = 'AbortError';
    d.reject(e);
  });
  let emit: (frame: { status: string; completed?: number; total?: number }) => void = () => {};

  const job = mount<void>({
    run: streamingRunner<void, { status: string; completed?: number; total?: number }>(
      (onData) => { emit = onData; return { promise: d.promise, abort }; },
      (frame) => ({ status: frame.status, completed: frame.completed, total: frame.total }),
    ),
  });

  await act(async () => { void job.current.start(); });
  await act(async () => { emit({ status: 'Downloading layer 3/8', completed: 3, total: 8 }); });
  expect(job.current.reading.status).toBe('Downloading layer 3/8');
  expect(job.current.percent).toBe(38);

  await act(async () => { job.current.abort(); await d.promise.catch(() => {}); });
  expect(abort).toHaveBeenCalled();
  expect(job.current.phase).toBe('stopped');
});
