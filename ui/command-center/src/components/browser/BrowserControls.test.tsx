/** @vitest-environment jsdom */
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
const { api } = vi.hoisted(() => ({ api: {
  getBrowserBookmarks: vi.fn(), getBrowserTabSets: vi.fn(), putBrowserBookmarks: vi.fn(), putBrowserTabSets: vi.fn(),
} }));
vi.mock('../../lib/api', () => ({ api }));
import { BrowserTabs } from './BrowserTabs';
import { BookmarksBar } from './BookmarksBar';
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root; let host: HTMLDivElement;
beforeEach(() => {
  host = document.createElement('div'); document.body.append(host); root = createRoot(host);
  api.getBrowserBookmarks.mockResolvedValue({ bookmarks: [{ url: 'https://example.com', title: 'Example', createdAt: '' }] });
  api.getBrowserTabSets.mockResolvedValue({ tabSets: [{ name: 'Work', tabs: [{ url: 'https://example.com', title: 'Example' }], createdAt: '' }] });
  api.putBrowserBookmarks.mockResolvedValue({}); api.putBrowserTabSets.mockResolvedValue({});
});
afterEach(() => { act(() => root.unmount()); host.remove(); vi.clearAllMocks(); });
function named(name: string) { return host.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!; }
function separate(button: HTMLButtonElement) { expect(button).toBeTruthy(); expect(button.parentElement!.closest('button, [role="button"]')).toBeNull(); }
it('exposes tab selection and close as independent native controls', async () => {
  const select = vi.fn(); const close = vi.fn();
  await act(async () => root.render(<BrowserTabs tabs={['a', 'b'].map(id => ({ id, label: id, webviewId: null, url: '', loading: false }))} activeTabId="a" closingTabId={null} onSelectTab={select} onCloseTab={close} onNewTab={vi.fn()} onCycleTab={vi.fn()} />));
  separate(named('Switch to tab: a')); separate(named('Close tab: a'));
  act(() => named('Close tab: a').click()); expect(close).toHaveBeenCalledWith('a'); expect(select).not.toHaveBeenCalled();
  act(() => named('Switch to tab: b').click()); expect(select).toHaveBeenCalledWith('b');
});
it('removing a bookmark never navigates, and neither control is nested', async () => {
  const navigate = vi.fn();
  await act(async () => root.render(<BookmarksBar currentUrl="" currentTitle="" openTabs={[]} onNavigate={navigate} onOpenInNewTab={vi.fn()} />));
  separate(named('Open bookmark: Example')); separate(named('Remove bookmark: Example'));
  await act(async () => named('Remove bookmark: Example').click());
  expect(navigate).not.toHaveBeenCalled(); expect(api.putBrowserBookmarks).toHaveBeenCalledWith([]);
});
