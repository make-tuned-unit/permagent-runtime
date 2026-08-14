/**
 * @vitest-environment jsdom
 *
 * The browser half of the contract: a batch must not die with the tab, the
 * session id comes from sessionStorage (no cookie, never cross-site), and drops
 * counted on a page that then closed are reported on the NEXT load — otherwise
 * the largest loss is also the least visible one.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createTracker, type SendResult, type Transport } from './index';

const ok: SendResult = { ok: true };

function recorder() {
  const calls: { body: Record<string, unknown>; unloading: boolean }[] = [];
  const transport: Transport = async (_endpoint, body, opts) => {
    calls.push({ body: JSON.parse(body), unloading: opts.unloading });
    return ok;
  };
  return { transport, calls };
}

beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  sessionStorage.clear();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('page lifecycle', () => {
  it('flushes what is queued when the page is hidden, marked as unloading', async () => {
    const r = recorder();
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 50,
      flushIntervalMs: 60_000,
      transport: r.transport,
      reportDrops: false,
    });

    tracker.trackEvent('almost_lost');
    expect(r.calls).toHaveLength(0); // nowhere near the batch size or the tick

    Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    await vi.advanceTimersByTimeAsync(0);

    expect(r.calls).toHaveLength(1);
    expect(r.calls[0]!.body.n).toBe('almost_lost');
    // The transport is told this is an unload, which is what selects
    // sendBeacon over fetch in the default transport.
    expect(r.calls[0]!.unloading).toBe(true);
  });

  it('also flushes on pagehide, which is what fires on a desktop navigation', async () => {
    const r = recorder();
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 50,
      flushIntervalMs: 60_000,
      transport: r.transport,
      reportDrops: false,
    });
    tracker.trackEvent('navigating_away');
    window.dispatchEvent(new Event('pagehide'));
    await vi.advanceTimersByTimeAsync(0);
    expect(r.calls.map((c) => c.body.n)).toEqual(['navigating_away']);
  });

  it('stamps every event with the sessionStorage session id', async () => {
    const r = recorder();
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      transport: r.transport,
      reportDrops: false,
    });
    tracker.trackEvent('with_session');
    await vi.advanceTimersByTimeAsync(0);
    const sid = sessionStorage.getItem('_pa_sid');
    expect(sid).toBeTruthy();
    expect(r.calls[0]!.body.s).toBe(sid);
  });
});

describe('drops that outlive the page', () => {
  it('carries a previous load’s drop count into the next load and reports it', async () => {
    // A tab closed mid-flush left this behind.
    localStorage.setItem('_pa_dropped', '4');

    const r = recorder();
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      transport: r.transport,
      reportDrops: true,
    });
    await vi.advanceTimersByTimeAsync(0);

    expect(r.calls).toHaveLength(1);
    expect(r.calls[0]!.body.n).toBe('permagent_client_dropped');
    expect(r.calls[0]!.body.d).toEqual({ count: 4, reason: 'previous_page' });
    expect(tracker.stats().dropped).toBe(4);
    expect(tracker.stats().droppedByReason.previous_page).toBe(4);
    // Consumed, so the next load does not double-report it.
    expect(localStorage.getItem('_pa_dropped')).toBe('0');
  });

  it('persists a drop so it survives the page that produced it', async () => {
    const transport: Transport = async () => ({ ok: false, retryable: true, error: 'offline' });
    const tracker = createTracker({
      endpoint: '/collect',
      batchSize: 1,
      maxAttempts: 1,
      transport,
      reportDrops: false,
    });
    tracker.trackEvent('doomed');
    await vi.advanceTimersByTimeAsync(10);
    expect(tracker.stats().dropped).toBe(1);
    expect(localStorage.getItem('_pa_dropped')).toBe('1');
  });
});
