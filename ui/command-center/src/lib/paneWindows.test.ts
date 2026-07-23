import { beforeEach, describe, expect, it } from 'vitest';
import { paneWindowLabel, stashPaneTab, takePaneTab } from './paneWindows';

describe('pane window state handoff', () => {
  beforeEach(() => {
    const values = new Map<string, string>();
    Object.defineProperty(globalThis, 'localStorage', {
      configurable: true,
      value: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
        removeItem: (key: string) => values.delete(key),
        clear: () => values.clear(),
      },
    });
  });

  it('uses unique, capability-scoped labels', () => {
    const first = paneWindowLabel('terminal');
    const second = paneWindowLabel('terminal');
    expect(first).toMatch(/^terminal-/);
    expect(second).not.toBe(first);
    expect(paneWindowLabel('browser')).toMatch(/^browser-/);
  });

  it('transfers a live terminal tab exactly once', () => {
    const tab = { id: 't1', label: 'server', sessionId: 'pty-1', cwd: '/work' };
    stashPaneTab('terminal-test', tab);
    expect(takePaneTab('terminal-test')).toEqual(tab);
    expect(takePaneTab('terminal-test')).toBeNull();
  });

  it('preserves the native webview identity for browser reparenting', () => {
    const tab = { id: 'b1', label: 'Docs', webviewId: 'browser-4', url: 'https://example.com', loading: false };
    stashPaneTab('browser-test', tab);
    expect(takePaneTab('browser-test')).toEqual(tab);
  });
});
