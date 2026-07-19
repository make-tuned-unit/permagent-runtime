/**
 * Brain-loop "View in Brain" store seam — focusBrainMemory stashes the target so
 * BrainView can consume the deep-link, and clearPendingBrainMemory resets it once
 * the memory is focused. `./api` is mocked (the chatStop pattern) so importing the
 * store touches no network; with no workspaces loaded, navigateToTool is a no-op,
 * so these assertions isolate the pending-target wiring.
 */

import { describe, expect, it, vi, beforeEach } from 'vitest';

vi.mock('./api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
}));

// Imported AFTER the mock is registered (vi.mock is hoisted above imports).
import { useCommandCenter } from './store';

beforeEach(() => {
  useCommandCenter.setState({ pendingBrainMemory: null, workspaces: [] });
});

describe('focusBrainMemory seam', () => {
  it('stashes the full target so BrainView can focus it', () => {
    const target = { id: 'm1', key: 'note:p:1', preview: { text: 'body', description: null } };
    useCommandCenter.getState().focusBrainMemory(target);
    expect(useCommandCenter.getState().pendingBrainMemory).toEqual(target);
  });

  it('accepts a bare key target (best-effort focus, no preview)', () => {
    useCommandCenter.getState().focusBrainMemory({ key: 'code:p:map' });
    expect(useCommandCenter.getState().pendingBrainMemory).toEqual({ key: 'code:p:map' });
  });

  it('clearPendingBrainMemory resets the target', () => {
    useCommandCenter.getState().focusBrainMemory({ key: 'code:p:map' });
    expect(useCommandCenter.getState().pendingBrainMemory).not.toBeNull();
    useCommandCenter.getState().clearPendingBrainMemory();
    expect(useCommandCenter.getState().pendingBrainMemory).toBeNull();
  });
});
