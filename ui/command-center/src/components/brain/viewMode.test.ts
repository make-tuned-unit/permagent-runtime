/** @vitest-environment jsdom */
/**
 * Brain's front door (J12).
 *
 * The 3D force-graph is the most internals-shaped surface in the app and it was
 * the first thing every user saw. List already has real dates, real search
 * highlighting and no undiscoverable interaction model — so List is the default
 * and Graph is the one you choose. The choice is remembered, because someone
 * who prefers the graph should have to say so once.
 *
 * The distinction these pin: only a DELIBERATE toggle is remembered. Search
 * flips the view to List on its own and flips it back when the query clears;
 * neither of those is the user telling us anything.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { readViewMode, rememberViewMode, VIEW_MODE_KEY } from './viewMode';

beforeEach(() => {
  localStorage.clear();
  vi.restoreAllMocks();
});

describe('Brain view-mode preference', () => {
  it('opens on the list for someone who has never chosen', () => {
    expect(readViewMode()).toBe('list');
  });

  it('remembers a deliberate choice of the graph', () => {
    rememberViewMode('graph');
    expect(localStorage.getItem(VIEW_MODE_KEY)).toBe('graph');
    expect(readViewMode()).toBe('graph');
  });

  it('remembers a deliberate choice of the list too', () => {
    rememberViewMode('graph');
    rememberViewMode('list');
    expect(readViewMode()).toBe('list');
  });

  it('falls back to the list rather than trusting a junk value', () => {
    localStorage.setItem(VIEW_MODE_KEY, 'nonsense');
    expect(readViewMode()).toBe('list');
  });

  it('survives storage being unavailable, which is not a reason to fail to open', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('denied'); });
    vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('denied'); });
    expect(readViewMode()).toBe('list');
    expect(() => rememberViewMode('graph')).not.toThrow();
  });
});
