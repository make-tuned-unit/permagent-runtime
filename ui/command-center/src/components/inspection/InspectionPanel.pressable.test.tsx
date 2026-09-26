/**
 * @vitest-environment jsdom
 *
 * dagK-pressable: the live event tail and the recent-memories rows expanded
 * their JSON/detail on click only, with no role, tabIndex or keyboard path.
 * Converted to `pressable` (see components/common/pressable.ts).
 */
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';
import { InspectionPanel } from './InspectionPanel';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function jsonResponse(body: unknown) {
  return { ok: true, json: async () => body } as Response;
}

const fetchAuthed = vi.fn((url: string) => {
  if (url.includes('recent-memories')) {
    return Promise.resolve(jsonResponse({
      memories: [{ id: 'mem-1', key: 'k', content: 'Henry learned the pressable pattern', wing: 'library', compaction_tier: null, created_at: null }],
    }));
  }
  if (url.includes('/activity/recent')) {
    return Promise.resolve(jsonResponse([
      { timestamp: new Date().toISOString(), source_surface: 'chat', type: 'message', payload: { ok: true } },
    ]));
  }
  if (url.includes('ingest-status')) {
    return Promise.resolve(jsonResponse({ paused: false, events_ingested: { always: 0, aggregated: 0 }, events_observed: { ephemeral: 0 } }));
  }
  return Promise.resolve(jsonResponse({}));
});
vi.mock('../../lib/api', () => ({ api: { fetchAuthed: (url: string) => fetchAuthed(url) } }));

let container: HTMLDivElement;
let root: Root;
const initialState = useCommandCenter.getState();

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({ ...initialState });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState(initialState);
});

async function render() {
  await act(async () => { root.render(<InspectionPanel onClose={() => {}} />); });
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

it('an event row is a keyboard-operable disclosure (role, tabIndex, aria-expanded)', async () => {
  await render();
  const row = container.querySelector('[aria-label^="Expand event"]') as HTMLElement;
  expect(row).toBeTruthy();
  expect(row.getAttribute('role')).toBe('button');
  expect(row.tabIndex).toBe(0);
  expect(row.getAttribute('aria-expanded')).toBe('false');
  await act(async () => {
    row.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
  });
  expect(row.getAttribute('aria-expanded')).toBe('true');
  expect(row.textContent).toContain('"ok": true');
});

it('a memory row is a keyboard-operable disclosure', async () => {
  await render();
  // Open the "Recent Ambient Memories" disclosure button first.
  const toggle = Array.from(container.querySelectorAll('button')).find(b => b.textContent?.includes('Recent Ambient Memories')) as HTMLButtonElement;
  expect(toggle).toBeTruthy();
  await act(async () => { toggle.click(); });

  const row = container.querySelector('[aria-label^="Expand memory"]') as HTMLElement;
  expect(row).toBeTruthy();
  expect(row.getAttribute('role')).toBe('button');
  expect(row.tabIndex).toBe(0);
  await act(async () => {
    row.dispatchEvent(new KeyboardEvent('keydown', { key: ' ', bubbles: true, cancelable: true }));
  });
  expect(row.getAttribute('aria-expanded')).toBe('true');
  expect(row.textContent).toContain('Henry learned the pressable pattern');
});
