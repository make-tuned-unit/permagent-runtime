/**
 * Regression tests for the wizard intent hand-off (2026-07 wiring audit).
 *
 * The onboarding "What will you build together?" step used to collect an intent
 * and drop it — the step claimed it would "prepare context for your first
 * conversation" but nothing consumed the text. These tests pin the real
 * hand-off: stash on wizard completion, take (one-shot) in the chat composer.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { stashWizardIntent, takeWizardIntent } from './wizardIntent';

// In-memory localStorage stub (the node test env has no DOM storage).
function installStorage(): Map<string, string> {
  const store = new Map<string, string>();
  vi.stubGlobal('localStorage', {
    getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
    setItem: (k: string, v: string) => { store.set(k, String(v)); },
    removeItem: (k: string) => { store.delete(k); },
    clear: () => store.clear(),
  });
  return store;
}

beforeEach(() => { installStorage(); });
afterEach(() => { vi.unstubAllGlobals(); });

describe('wizardIntent hand-off', () => {
  it('round-trips: what the wizard stashes is what the composer takes', () => {
    stashWizardIntent('Build a SaaS product from scratch');
    expect(takeWizardIntent()).toBe('Build a SaaS product from scratch');
  });

  it('is one-shot: a second take returns null (never re-fills the composer)', () => {
    stashWizardIntent('Automate my workflow');
    expect(takeWizardIntent()).toBe('Automate my workflow');
    expect(takeWizardIntent()).toBeNull();
  });

  it('trims surrounding whitespace before stashing', () => {
    stashWizardIntent('   Learn Rust   ');
    expect(takeWizardIntent()).toBe('Learn Rust');
  });

  it('no-ops on a blank/whitespace intent (nothing to hand off)', () => {
    stashWizardIntent('');
    expect(takeWizardIntent()).toBeNull();
    stashWizardIntent('    ');
    expect(takeWizardIntent()).toBeNull();
  });

  it('take with nothing stashed returns null', () => {
    expect(takeWizardIntent()).toBeNull();
  });

  it('survives localStorage throwing (private mode / disabled) without raising', () => {
    vi.stubGlobal('localStorage', {
      getItem: () => { throw new Error('denied'); },
      setItem: () => { throw new Error('denied'); },
      removeItem: () => { throw new Error('denied'); },
    });
    expect(() => stashWizardIntent('x')).not.toThrow();
    expect(takeWizardIntent()).toBeNull();
  });
});
