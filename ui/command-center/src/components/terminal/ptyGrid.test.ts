// @vitest-environment jsdom
import { describe, it, expect, vi } from 'vitest';
import {
  FALLBACK_PTY_GRID,
  advertisedGrid,
  containerCanFit,
  fitVisibleTerminal,
  remeasureXterm,
  subscribeTerminalFonts,
} from './ptyGrid';

describe('containerCanFit', () => {
  it('rejects a collapsed box — FitAddon would floor this to 2×1', () => {
    expect(containerCanFit(null)).toBe(false);
    expect(containerCanFit(undefined)).toBe(false);
    expect(containerCanFit({ offsetWidth: 0, offsetHeight: 0 })).toBe(false);
    expect(containerCanFit({ offsetWidth: 800, offsetHeight: 0 })).toBe(false);
    expect(containerCanFit({ offsetWidth: 0, offsetHeight: 600 })).toBe(false);
  });

  it('accepts a laid-out pane', () => {
    expect(containerCanFit({ offsetWidth: 800, offsetHeight: 600 })).toBe(true);
  });
});

describe('advertisedGrid', () => {
  it('rejects FitAddon’s collapsed 2×1 — that is the status-on-prompt paint', () => {
    expect(advertisedGrid({ cols: 2, rows: 1 })).toBeNull();
    expect(advertisedGrid({ cols: 80, rows: 1 })).toBeNull();
    expect(advertisedGrid({ cols: 1, rows: 24 })).toBeNull();
  });

  it('passes through a real grid, including the fallback 80×24', () => {
    expect(advertisedGrid({ cols: 217, rows: 48 })).toEqual({ cols: 217, rows: 48 });
    expect(advertisedGrid(FALLBACK_PTY_GRID)).toEqual({ cols: 80, rows: 24 });
  });
});

describe('fitVisibleTerminal', () => {
  it('does not call fit() on a 0-box — the old unguarded path resized xterm to 2×1', () => {
    const fit = vi.fn();
    expect(fitVisibleTerminal({ fit }, { offsetWidth: 0, offsetHeight: 0 })).toBe(false);
    expect(fit).not.toHaveBeenCalled();
  });

  it('fits when the pane has a real size', () => {
    const fit = vi.fn();
    expect(fitVisibleTerminal({ fit }, { offsetWidth: 800, offsetHeight: 600 })).toBe(true);
    expect(fit).toHaveBeenCalledTimes(1);
  });

  it('swallows a fit() throw during layout transitions', () => {
    const fit = vi.fn(() => {
      throw new Error('layout');
    });
    expect(fitVisibleTerminal({ fit }, { offsetWidth: 800, offsetHeight: 600 })).toBe(false);
  });
});

describe('remeasureXterm', () => {
  it('re-assigns fontFamily so CharSizeService remeasures — and writes nothing', () => {
    const term = { options: { fontFamily: '"JetBrains Mono", monospace' } };
    remeasureXterm(term);
    expect(term.options.fontFamily).toBe('"JetBrains Mono", monospace');
  });

  it('is a no-op when fontFamily was never set', () => {
    const term = { options: {} };
    expect(() => remeasureXterm(term)).not.toThrow();
  });
});

describe('subscribeTerminalFonts', () => {
  it('runs on fonts.ready and not after unsubscribe', async () => {
    const original = document.fonts;
    let settleReady: (value?: unknown) => void = () => {};
    const ready = new Promise(resolve => {
      settleReady = resolve;
    });
    const listeners: Array<() => void> = [];
    Object.defineProperty(document, 'fonts', {
      configurable: true,
      value: {
        ready,
        addEventListener: (_type: string, listener: () => void) => {
          listeners.push(listener);
        },
        removeEventListener: (_type: string, listener: () => void) => {
          const i = listeners.indexOf(listener);
          if (i >= 0) listeners.splice(i, 1);
        },
      },
    });
    try {
      const cb = vi.fn();
      const unsub = subscribeTerminalFonts(cb);
      expect(cb).not.toHaveBeenCalled();
      settleReady();
      await ready;
      await Promise.resolve();
      expect(cb).toHaveBeenCalledTimes(1);
      unsub();
      for (const listener of listeners) listener();
      expect(cb).toHaveBeenCalledTimes(1);
    } finally {
      Object.defineProperty(document, 'fonts', { configurable: true, value: original });
    }
  });
});
