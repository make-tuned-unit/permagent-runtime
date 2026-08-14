/**
 * Tracker semantics, in a Node environment (no window, no document, no
 * localStorage) — which is half the contract: the same module has to work in a
 * server-side render or a CLI without touching the DOM.
 *
 * The transport is injected, so these are pure-logic tests over the queue,
 * the retry ladder and the drop accounting. Nothing here hits a network.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import {
  backoffDelay,
  createTracker,
  isRetryableStatus,
  sanitizeProperties,
  type SendResult,
  type Transport,
} from './index';

/** Collects every body handed to the transport and replies with a script of
 *  results (last result repeats). */
function scriptedTransport(results: SendResult[]) {
  const bodies: string[] = [];
  const unloading: boolean[] = [];
  let i = 0;
  const transport: Transport = async (_endpoint, body, opts) => {
    bodies.push(body);
    unloading.push(opts.unloading);
    const r = results[Math.min(i, results.length - 1)];
    i += 1;
    return r;
  };
  return { transport, bodies, unloading, names: () => bodies.map((b) => JSON.parse(b).n) };
}

const ok: SendResult = { ok: true };
const down: SendResult = { ok: false, retryable: true, error: 'offline' };
const badRequest: SendResult = { ok: false, retryable: false, status: 400, error: 'HTTP 400' };

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('trackEvent batching', () => {
  it('holds events until the batch fills, then sends them in order', async () => {
    const t = scriptedTransport([ok]);
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 3,
      flushIntervalMs: 5000,
      transport: t.transport,
      reportDrops: false,
    });

    tracker.trackEvent('a');
    tracker.trackEvent('b');
    await vi.advanceTimersByTimeAsync(0);
    expect(t.bodies).toHaveLength(0); // under the batch size, before the tick
    expect(tracker.stats().queued).toBe(2);

    tracker.trackEvent('c');
    await vi.advanceTimersByTimeAsync(0);
    expect(t.names()).toEqual(['a', 'b', 'c']);
    expect(tracker.stats().sent).toBe(3);
    expect(tracker.stats().queued).toBe(0);
  });

  it('flushes on the interval when the batch never fills', async () => {
    const t = scriptedTransport([ok]);
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 10,
      flushIntervalMs: 5000,
      transport: t.transport,
      reportDrops: false,
    });

    tracker.trackEvent('lonely');
    await vi.advanceTimersByTimeAsync(4999);
    expect(t.bodies).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(2);
    expect(t.names()).toEqual(['lonely']);
    expect(tracker.stats().sent).toBe(1);
  });

  it('never throws out of trackEvent, even when the transport rejects', async () => {
    const transport: Transport = async () => {
      throw new Error('boom');
    };
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      maxAttempts: 1,
      transport,
      reportDrops: false,
    });
    expect(() => tracker.trackEvent('safe')).not.toThrow();
    await vi.advanceTimersByTimeAsync(10);
    // The throw became a counted drop, not an unhandled rejection.
    expect(tracker.stats().dropped).toBe(1);
  });

  it('sanitizes properties the collector would refuse to group by', () => {
    const cleaned = sanitizeProperties({
      store: 'sobeys',
      count: 3,
      flag: true,
      nested: { a: 1 } as unknown as string,
      list: [1, 2] as unknown as string,
      nothing: null,
      long: 'x'.repeat(400),
      nan: Number.NaN,
    });
    expect(cleaned).toEqual({
      store: 'sobeys',
      count: 3,
      flag: true,
      long: 'x'.repeat(256),
    });
    expect(sanitizeProperties(undefined)).toBeNull();
  });
});

describe('retry', () => {
  it('retries a transient failure with a growing delay and preserves order', async () => {
    // Every event fails its FIRST attempt and succeeds on the second, so the
    // retry ladder is exercised per event rather than per request.
    const seen = new Map<string, number>();
    const bodies: string[] = [];
    const t = {
      bodies,
      names: () => bodies.map((b) => JSON.parse(b).n as string),
      transport: (async (_endpoint, body) => {
        bodies.push(body);
        const name = JSON.parse(body).n as string;
        const n = (seen.get(name) ?? 0) + 1;
        seen.set(name, n);
        return n === 1 ? down : ok;
      }) as Transport,
    };
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 2,
      flushIntervalMs: 60_000,
      retryBaseMs: 1000,
      transport: t.transport,
      reportDrops: false,
    });

    tracker.trackEvent('first');
    tracker.trackEvent('second');
    await vi.advanceTimersByTimeAsync(0);
    expect(t.names()).toEqual(['first', 'second']); // attempt 1: both fail
    expect(tracker.stats().sent).toBe(0);
    expect(tracker.stats().retries).toBe(2);

    // Backoff is jittered, so allow the whole first-attempt window.
    await vi.advanceTimersByTimeAsync(2000);
    expect(t.names()).toEqual(['first', 'second', 'first', 'second']);
    expect(tracker.stats().sent).toBe(2);
    expect(tracker.stats().dropped).toBe(0);
  });

  it('gives up after maxAttempts and COUNTS the loss', async () => {
    const t = scriptedTransport([down]);
    const dropped: string[] = [];
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      maxAttempts: 3,
      retryBaseMs: 10,
      retryMaxMs: 20,
      transport: t.transport,
      reportDrops: false,
      onDrop: (events, reason) => {
        for (const e of events) dropped.push(`${e.n}:${reason}`);
      },
    });

    tracker.trackEvent('doomed');
    await vi.advanceTimersByTimeAsync(500);

    expect(t.bodies.filter((b) => JSON.parse(b).n === 'doomed')).toHaveLength(3);
    expect(tracker.stats().dropped).toBe(1);
    expect(tracker.stats().droppedByReason.retries_exhausted).toBe(1);
    expect(dropped).toEqual(['doomed:retries_exhausted']);
    expect(tracker.stats().lastError).toBe('offline');
  });

  it('does not retry a rejection the endpoint will never accept', async () => {
    const t = scriptedTransport([badRequest]);
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      maxAttempts: 5,
      retryBaseMs: 10,
      transport: t.transport,
      reportDrops: false,
    });

    tracker.trackEvent('malformed');
    await vi.advanceTimersByTimeAsync(1000);

    // One attempt only: eight seconds of backoff would not turn a 400 into a 200.
    expect(t.bodies).toHaveLength(1);
    expect(tracker.stats().droppedByReason.rejected).toBe(1);
  });

  it('classifies statuses the way the retry ladder depends on', () => {
    expect(isRetryableStatus(500)).toBe(true);
    expect(isRetryableStatus(429)).toBe(true);
    expect(isRetryableStatus(408)).toBe(true);
    expect(isRetryableStatus(400)).toBe(false);
    expect(isRetryableStatus(404)).toBe(false);
  });

  it('backs off exponentially and stays inside the cap', () => {
    const noJitter = () => 0.5;
    expect(backoffDelay(1, 1000, 30_000, noJitter)).toBe(1000);
    expect(backoffDelay(2, 1000, 30_000, noJitter)).toBe(2000);
    expect(backoffDelay(3, 1000, 30_000, noJitter)).toBe(4000);
    expect(backoffDelay(10, 1000, 30_000, noJitter)).toBe(30_000);
    // Jitter never turns into a negative or unbounded delay.
    for (const r of [0, 0.999]) {
      const d = backoffDelay(4, 1000, 30_000, () => r);
      expect(d).toBeGreaterThanOrEqual(0);
      expect(d).toBeLessThanOrEqual(30_000 * 1.25);
    }
  });
});

describe('drop accounting', () => {
  it('bounds the queue and counts what overflow discarded', async () => {
    const t = scriptedTransport([down]);
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 2,
      maxQueueEvents: 4,
      maxAttempts: 99,
      retryBaseMs: 100_000, // park the retry so the queue actually fills
      transport: t.transport,
      reportDrops: false,
    });

    for (let i = 0; i < 10; i += 1) tracker.trackEvent(`e${i}`);
    await vi.advanceTimersByTimeAsync(0);

    expect(tracker.stats().queued).toBeLessThanOrEqual(4);
    expect(tracker.stats().droppedByReason.queue_overflow).toBeGreaterThan(0);
    // Nothing vanishes unaccounted for: everything is sent, queued or counted.
    const s = tracker.stats();
    expect(s.sent + s.dropped + s.queued).toBe(10);
  });

  it('reports exhausted drops back into analytics as an event', async () => {
    // First event fails forever; the drop report that follows succeeds.
    let sawDropReport = false;
    const transport: Transport = async (_e, body) => {
      const parsed = JSON.parse(body);
      if (parsed.n === 'permagent_client_dropped') {
        sawDropReport = true;
        expect(parsed.d).toEqual({ count: 1, reason: 'retries_exhausted' });
        return ok;
      }
      return down;
    };
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      maxAttempts: 2,
      retryBaseMs: 10,
      transport,
      reportDrops: true,
    });

    tracker.trackEvent('doomed');
    await vi.advanceTimersByTimeAsync(5000);

    expect(sawDropReport).toBe(true);
    // The report itself must not manufacture further reports.
    expect(tracker.stats().droppedByReason.retries_exhausted).toBe(1);
  });
});

describe('node environment', () => {
  it('runs with no window, document or localStorage', async () => {
    expect(typeof window).toBe('undefined');
    const t = scriptedTransport([ok]);
    const tracker = createTracker({
      endpoint: 'https://example.test/collect/abc',
      batchSize: 1,
      transport: t.transport,
      reportDrops: false,
    });
    tracker.trackEvent('server_side', { where: 'node' });
    tracker.trackPageview('/api/thing');
    await vi.advanceTimersByTimeAsync(10);

    const sent = t.bodies.map((b) => JSON.parse(b));
    expect(sent[0]).toMatchObject({ k: 'ev', n: 'server_side', p: '/', s: null });
    expect(sent[1]).toMatchObject({ k: 'pv', p: '/api/thing', n: null });
    await tracker.shutdown();
  });
});
