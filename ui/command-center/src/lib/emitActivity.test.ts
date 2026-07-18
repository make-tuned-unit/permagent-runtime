/**
 * emitActivity — fire-and-forget IPC seam tests.
 *
 * The regression this locks down: the `invoke()` promise must be returned into
 * the `.then` chain so a daemon-side rejection (4xx/5xx, missing token) lands
 * in the final `.catch` instead of escaping as an unhandled promise rejection.
 * `@tauri-apps/api/core` is mocked, so the dynamic import resolves and the
 * mocked `invoke` plays the daemon.
 */

import { describe, expect, it, vi, beforeEach, afterEach, type MockInstance } from 'vitest';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));

import { emitActivity } from './emitActivity';

describe('emitActivity', () => {
  let debugSpy: MockInstance;

  beforeEach(() => {
    invoke.mockReset();
    debugSpy = vi.spyOn(console, 'debug').mockImplementation(() => {});
  });

  afterEach(() => {
    debugSpy.mockRestore();
  });

  it('forwards event type, surface, and payload over the emit_activity IPC', async () => {
    invoke.mockResolvedValue({ accepted: true });
    emitActivity('inbox_opened', 'inbox', { source: 'test' });
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(invoke).toHaveBeenCalledWith('emit_activity', {
      event_type: 'inbox_opened',
      source_surface: 'inbox',
      payload: { source: 'test' },
    });
    expect(debugSpy).not.toHaveBeenCalled();
  });

  it('defaults the payload to an empty object', async () => {
    invoke.mockResolvedValue({ accepted: true });
    emitActivity('grow_opened', 'grow');
    await vi.waitFor(() => expect(invoke).toHaveBeenCalledTimes(1));
    expect(invoke.mock.calls[0][1]).toMatchObject({ payload: {} });
  });

  it('catches a daemon rejection inside the chain (no unhandled promise rejection)', async () => {
    const unhandled: unknown[] = [];
    const onUnhandled = (reason: unknown) => {
      unhandled.push(reason);
    };
    process.on('unhandledRejection', onUnhandled);
    try {
      invoke.mockRejectedValue(new Error('daemon returned 400'));
      expect(() => emitActivity('brain_opened', 'brain')).not.toThrow();
      // The chained .catch must observe the rejection…
      await vi.waitFor(() => expect(debugSpy).toHaveBeenCalled());
      // …and give the unhandled-rejection queue a macrotask to (not) fire.
      await new Promise(resolve => setImmediate(resolve));
      await new Promise(resolve => setImmediate(resolve));
      expect(unhandled).toEqual([]);
    } finally {
      process.off('unhandledRejection', onUnhandled);
    }
  });
});
