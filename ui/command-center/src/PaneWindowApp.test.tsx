// @vitest-environment jsdom
import { StrictMode } from 'react';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as paneWindows from './lib/paneWindows';
import PaneWindowApp from './PaneWindowApp';

const mocks = vi.hoisted(() => ({
  browserTabs: [] as unknown[],
  closeHandler: undefined as ((event: { preventDefault: () => void }) => void) | undefined,
  destroy: vi.fn<[], Promise<void>>(),
  invoke: vi.fn(),
  removeItem: vi.fn(),
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke: mocks.invoke }));
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({
    label: 'pane-window-test',
    destroy: mocks.destroy,
    isMinimized: vi.fn().mockResolvedValue(false),
    onCloseRequested: vi.fn(async (handler) => {
      mocks.closeHandler = handler;
      return vi.fn();
    }),
    onResized: vi.fn(async () => vi.fn()),
  }),
}));
vi.mock('./components/browser', async () => {
  const { forwardRef, useImperativeHandle } = await import('react');
  return {
    Browser: forwardRef(function MockBrowser(props: { initialTab: unknown }, ref) {
      mocks.browserTabs.push(props.initialTab);
      useImperativeHandle(ref, () => ({
        getActiveTab: () => props.initialTab,
        getAllTabs: () => [props.initialTab],
      }));
      return <div data-testid="browser" />;
    }),
  };
});
vi.mock('./components/terminal/TerminalManager', async () => {
  const { forwardRef } = await import('react');
  return { TerminalManager: forwardRef(function MockTerminal() { return <div />; }) };
});
vi.mock('./styles/useTheme', () => ({ useTheme: () => ({ gradient: { workspace: '#000' } }) }));
vi.mock('./lib/paneWindows', async importOriginal => ({
  ...await importOriginal<typeof import('./lib/paneWindows')>(),
  emitRedock: vi.fn().mockResolvedValue(undefined),
}));

let root: Root;
let container: HTMLDivElement;

async function renderBrowser() {
  await act(async () => {
    root.render(<StrictMode><PaneWindowApp /></StrictMode>);
  });
}

beforeEach(() => {
  const values = new Map<string, string>();
  mocks.removeItem.mockReset().mockImplementation((key: string) => values.delete(key));
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: mocks.removeItem,
      clear: () => values.clear(),
    },
  });
  history.replaceState(null, '', '?view=pane&kind=browser&owner=browser-test');
  mocks.browserTabs.length = 0;
  mocks.closeHandler = undefined;
  mocks.destroy.mockReset().mockResolvedValue(undefined);
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

describe('PaneWindowApp', () => {
  it('consumes a pane handoff once after commit under StrictMode double-invocation', async () => {
    const tab = { id: 'b1', label: 'Docs', webviewId: 'browser-4', url: 'https://example.com', loading: false };
    paneWindows.stashPaneTab('browser-test', tab);
    await renderBrowser();

    expect(mocks.removeItem).toHaveBeenCalledTimes(1);
    expect(mocks.browserTabs).toEqual([tab, tab]);
    expect(container.querySelector('[data-testid="browser"]')).not.toBeNull();
  });

  it('redocks the open site into the app and destroys the window on close', async () => {
    const tab = { id: 'b1', label: 'Docs', webviewId: 'browser-4', url: 'https://example.com', loading: false };
    paneWindows.stashPaneTab('browser-test', tab);
    await renderBrowser();

    await act(async () => {
      mocks.closeHandler?.({ preventDefault: vi.fn() });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // Close = hand the content back: the loaded tab reparents into main (its
    // page survives as an in-app tab) and the pane window always dies.
    expect(mocks.invoke).toHaveBeenCalledWith('reparent_browser', {
      webviewId: 'browser-4',
      windowLabel: 'main',
    });
    expect(paneWindows.emitRedock).toHaveBeenCalledWith('browser', tab);
    expect(mocks.invoke).not.toHaveBeenCalledWith('destroy_pane_window', expect.anything());
    expect(mocks.destroy).toHaveBeenCalledTimes(1);
  });

  it('still destroys the window when the redock fails', async () => {
    // Regression: the close handler used to throw before destroy(), stranding
    // a window the user had asked to dismiss. A failed handoff now falls back
    // to the native teardown and the window dies regardless.
    const tab = { id: 'b1', label: 'Docs', webviewId: 'browser-4', url: 'https://example.com', loading: false };
    paneWindows.stashPaneTab('browser-test', tab);
    mocks.invoke.mockRejectedValue(new Error('transient reparent failure'));
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await renderBrowser();

    await act(async () => {
      mocks.closeHandler?.({ preventDefault: vi.fn() });
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    // The stranded webview goes to the atomic native teardown instead.
    expect(mocks.invoke).toHaveBeenCalledWith('destroy_pane_window', {
      windowLabel: 'pane-window-test',
      webviewIds: ['browser-4'],
    });
    expect(mocks.destroy).toHaveBeenCalledTimes(1);
  });
});
