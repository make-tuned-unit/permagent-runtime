/**
 * @vitest-environment jsdom
 *
 * Teach once, then be quiet.
 *
 * A canvas key that opens every visit is a nag; one that never opens by itself
 * teaches nobody. So the rule is: open the first time this canvas is met, shut
 * for good once the user has said so — and each canvas remembers separately,
 * because learning the World teaches you nothing about the Brain's graph.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  canvasLegendStorageKey,
  readLegendOpen,
  rememberLegendOpen,
} from './legendMemory';

beforeEach(() => localStorage.clear());

describe('canvas legend memory', () => {
  it('opens on a first visit, when nothing has been remembered', () => {
    expect(readLegendOpen('world')).toBe(true);
  });

  it('stays shut once dismissed', () => {
    rememberLegendOpen('world', false);
    expect(readLegendOpen('world')).toBe(false);
  });

  it('opens again if the user reopens it', () => {
    rememberLegendOpen('world', false);
    rememberLegendOpen('world', true);
    expect(readLegendOpen('world')).toBe(true);
  });

  it('remembers each canvas separately', () => {
    rememberLegendOpen('world', false);
    expect(readLegendOpen('brain-graph')).toBe(true);
    expect(readLegendOpen('people-graph')).toBe(true);
    expect(canvasLegendStorageKey('world')).not.toBe(canvasLegendStorageKey('brain-graph'));
  });

  it('opens rather than fails when storage is unavailable', () => {
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('blocked site data');
    });
    const setItem = vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
      throw new Error('blocked site data');
    });
    expect(readLegendOpen('world')).toBe(true);
    expect(() => rememberLegendOpen('world', false)).not.toThrow();
    getItem.mockRestore();
    setItem.mockRestore();
  });
});
