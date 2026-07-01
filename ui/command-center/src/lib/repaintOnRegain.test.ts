import { describe, it, expect, vi, afterEach } from 'vitest';
import { onRepaintRegain, fireRepaint } from './repaintOnRegain';

// Runs under the default node env (no DOM): onRepaintRegain's lazy wiring
// no-ops when `window` is undefined, so the subscriber registry is exercised
// in isolation.

describe('repaintOnRegain', () => {
  afterEach(() => {
    // Drain any leftover subscribers between tests.
    fireRepaint();
  });

  it('fires a subscribed callback on regain', () => {
    const cb = vi.fn();
    const off = onRepaintRegain(cb);
    fireRepaint();
    expect(cb).toHaveBeenCalledTimes(1);
    off();
  });

  it('stops firing after unsubscribe', () => {
    const cb = vi.fn();
    const off = onRepaintRegain(cb);
    off();
    fireRepaint();
    expect(cb).not.toHaveBeenCalled();
  });

  it('isolates a throwing callback from the others', () => {
    const bad = vi.fn(() => {
      throw new Error('stale surface');
    });
    const good = vi.fn();
    const offBad = onRepaintRegain(bad);
    const offGood = onRepaintRegain(good);
    expect(() => fireRepaint()).not.toThrow();
    expect(bad).toHaveBeenCalledTimes(1);
    expect(good).toHaveBeenCalledTimes(1);
    offBad();
    offGood();
  });

  it('fires all current subscribers each regain', () => {
    const a = vi.fn();
    const b = vi.fn();
    const offA = onRepaintRegain(a);
    const offB = onRepaintRegain(b);
    fireRepaint();
    fireRepaint();
    expect(a).toHaveBeenCalledTimes(2);
    expect(b).toHaveBeenCalledTimes(2);
    offA();
    offB();
  });
});
