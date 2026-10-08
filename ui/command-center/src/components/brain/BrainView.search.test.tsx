/**
 * @vitest-environment jsdom
 *
 * Brain search wiring — the search box must call GET /api/brain/search and
 * drive the list + graph from its ranked results, not client-side filtering
 * over whatever happened to be loaded.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const sceneFocus = vi.fn();
const sceneClearFocus = vi.fn();
const sceneSetSearch = vi.fn();

class FakeResizeObserver {
  observe() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

vi.mock('./BrainScene', () => ({
  BrainScene: vi.fn().mockImplementation(() => ({
    setData: vi.fn(),
    setSearch: sceneSetSearch,
    setTypeFilter: vi.fn(),
    setTimeRange: vi.fn(),
    focusSearchHit: sceneFocus,
    clearSearchFocus: sceneClearFocus,
    setActive: vi.fn(),
    setSelectedId: vi.fn(),
    setContentInsets: vi.fn(),
    resize: vi.fn(),
    dispose: vi.fn(),
  })),
}));

vi.mock('../mobius/Mobius', () => ({
  Mobius: () => null,
}));

vi.mock('../../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/api')>();
  return {
    ...actual,
    api: {
      ...actual.api,
      searchBrain: vi.fn(),
      getBrainMemories: vi.fn(() => Promise.resolve({ memories: [], total: 0, has_more: false })),
    },
    apiFetch: vi.fn(),
  };
});

vi.mock('../world/shared/worldEvents', () => ({
  subscribeWorldEvents: () => () => {},
}));

import { BrainView } from './BrainView';
import { api, apiFetch } from '../../lib/api';
import { useCommandCenter } from '../../lib/store';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);
const searchBrainMock = vi.mocked(api.searchBrain);
const getBrainMemoriesMock = vi.mocked(api.getBrainMemories);

const graphPayload = {
  self: { name: 'Agent', id: 'self' },
  entities: [{ id: 'e:1', type: 'person', name: 'Alice', note: 'teammate', fields: [] }],
  edges: [],
  memories: [{
    id: 'mem-loaded',
    key: 'note:loaded',
    text: 'already loaded memory about alice',
    description: 'Loaded note',
    ent: ['e:1'],
    age: 0.2,
    weight: 0.8,
    timestamp: '2026-08-01T12:00:00+00:00',
  }],
};

const searchHit = {
  source: 'memory',
  id: 'spectral:0',
  preview: 'caroline wrote the deployment runbook yesterday',
  score: 0.91,
  timestamp: '2026-08-16T09:00:00+00:00',
  session_id: null,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.useFakeTimers();
  sceneFocus.mockReset();
  sceneClearFocus.mockReset();
  sceneSetSearch.mockReset();
  apiFetchMock.mockReset();
  searchBrainMock.mockReset();
  getBrainMemoriesMock.mockReset();
  getBrainMemoriesMock.mockResolvedValue({ memories: [], total: 0, has_more: false });

  apiFetchMock.mockImplementation((url: string) => {
    if (url.startsWith('/api/brain/graph')) return Promise.resolve(graphPayload);
    if (url === '/api/projects') return Promise.resolve([]);
    return Promise.resolve({});
  });

  searchBrainMock.mockResolvedValue({
    results: [searchHit],
    total: 1,
    query: 'caroline',
    offset: 0,
    limit: 50,
    fts_count: 0,
    spectral_count: 1,
    dedup_count: 0,
  });

  useCommandCenter.setState({ pendingBrainMemory: null, setPendingProjectNavigation: vi.fn() });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function renderBrain() {
  await act(async () => {
    root.render(<BrainView />);
  });
  await act(async () => {
    await Promise.resolve();
  });
}

function searchInput(): HTMLInputElement {
  const input = container.querySelector('input[placeholder*="try a name or project"]');
  expect(input).toBeTruthy();
  return input as HTMLInputElement;
}

function viewToggle(mode: 'graph' | 'list'): HTMLButtonElement {
  const btn = Array.from(container.querySelectorAll('button'))
    .find(b => b.textContent?.trim().toUpperCase() === mode.toUpperCase());
  expect(btn).toBeTruthy();
  return btn as HTMLButtonElement;
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

async function typeSearch(query: string) {
  const input = searchInput();
  await act(async () => {
    setInputValue(input, query);
    vi.advanceTimersByTime(300);
    await Promise.resolve();
    await Promise.resolve();
  });
}

async function clearSearch() {
  const input = searchInput();
  await act(async () => {
    setInputValue(input, '');
    vi.advanceTimersByTime(300);
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('Brain search guards', () => {
  it('1 — non-empty query calls GET /api/brain/search', async () => {
    await renderBrain();
    await typeSearch('caroline');

    expect(searchBrainMock).toHaveBeenCalledWith({ q: 'caroline' });
    expect(String(searchBrainMock.mock.calls[0]?.[0]?.q)).toBe('caroline');
  });

  it('2 — endpoint results populate the list, not a filter over pre-loaded rows', async () => {
    await renderBrain();
    await typeSearch('caroline');

    expect(container.textContent).toContain('caroline wrote the deployment runbook');
    expect(container.textContent).not.toContain('already loaded memory about alice');
    expect(getBrainMemoriesMock).not.toHaveBeenCalledWith(expect.objectContaining({ q: 'caroline' }));
  });

  it('3 — zero results and failed search render different states; error names a reason', async () => {
    await renderBrain();

    searchBrainMock.mockResolvedValueOnce({
      results: [],
      total: 0,
      query: 'caroline',
      offset: 0,
      limit: 50,
      fts_count: 0,
      spectral_count: 0,
      dedup_count: 0,
    });
    await typeSearch('caroline');
    expect(container.textContent).toContain('No memories match "caroline"');
    expect(container.textContent).not.toContain('Could not search your Brain');

    searchBrainMock.mockRejectedValueOnce(new Error('daemon offline'));
    await clearSearch();
    await typeSearch('caroline');
    expect(container.textContent).toContain('Could not search your Brain: daemon offline');
    expect(container.textContent).not.toContain('No memories match "caroline"');
  });

  it('4 — clearing the query restores the previous view mode', async () => {
    await renderBrain();
    await act(async () => {
      viewToggle('graph').dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    await typeSearch('caroline');
    expect(container.textContent).toContain('caroline wrote the deployment runbook');
    expect(container.textContent).toContain('matching "caroline"');

    await clearSearch();
    expect(container.textContent).not.toContain('matching "caroline"');
    // "Selected" is still the lit fill, but it is now carried on the button
    // primitive's `--pa-btn-bg` custom property rather than an inline
    // `background` an inline style could never give a hover or pressed state.
    expect(viewToggle('graph').style.getPropertyValue('--pa-btn-bg')).not.toBe('transparent');
  });

  it('5 — results assertion fails on an empty result set (non-vacuous floor)', async () => {
    await renderBrain();
    await typeSearch('caroline');

    expect(searchBrainMock).toHaveBeenCalled();
    const lastCall = searchBrainMock.mock.results[searchBrainMock.mock.results.length - 1];
    const response = await lastCall?.value;
    expect(response?.results?.length).toBeGreaterThan(0);
    expect(response?.results?.[0]?.preview).toContain('caroline');
  });

  it('Enter opens the top ranked search hit in the side panel', async () => {
    await renderBrain();
    await typeSearch('caroline');

    const input = searchInput();
    await act(async () => {
      input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });

    expect(container.textContent).toContain('caroline wrote the deployment runbook yesterday');
  });
});

const savedNote = {
  ...graphPayload.memories[0], id: 'saved-note-1', key: 'note:qa:meeting & tail',
  text: 'Full saved note. No purchase is authorized. Final review stays pending.',
  description: null, source: 'permagent.note', layer: 'full',
};

async function focusPreview(key = savedNote.key, text = 'Original note preview', id?: string) {
  await act(async () => {
    useCommandCenter.setState({ pendingBrainMemory: { key, id, preview: { text } } });
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe('Brain saved-memory preview resolution', () => {
  it('reads the full saved memory by encoded key and shows authoritative source without claiming graph placement', async () => {
    apiFetchMock.mockImplementation((url: string) => {
      if (url.startsWith('/api/brain/memory?')) return Promise.resolve(savedNote);
      if (url.startsWith('/api/brain/graph')) return Promise.resolve(graphPayload);
      return Promise.resolve([]);
    });
    await renderBrain();
    await focusPreview();
    expect(apiFetchMock).toHaveBeenCalledWith(
      `/api/brain/memory?key=${encodeURIComponent(savedNote.key)}`, expect.objectContaining({ signal: expect.any(AbortSignal) }),
    );
    expect(container.textContent).toContain(savedNote.text);
    expect(container.textContent).toContain('from a note');
    expect(container.textContent).not.toContain('source unknown');
    expect(container.textContent).toContain('Saved memory — outside current graph');
    expect(container.textContent).not.toContain('saved memory not verified');
  });

  it.each(['not found', 'wrong key', 'wrong id'] as const)('keeps the truthful preview when lookup returns %s', async failure => {
    apiFetchMock.mockImplementation((url: string) => {
      if (url.startsWith('/api/brain/memory?')) {
        if (failure === 'not found') return Promise.reject(new Error('HTTP 404'));
        return Promise.resolve({ ...savedNote, ...(failure === 'wrong key' ? { key: 'note:another' } : { id: 'another-id' }) });
      }
      if (url.startsWith('/api/brain/graph')) return Promise.resolve(graphPayload);
      return Promise.resolve([]);
    });
    await renderBrain();
    await focusPreview(savedNote.key, 'Original note preview', savedNote.id);
    expect(container.textContent).toContain('Original note preview');
    expect(container.textContent).toContain('Saved memory unavailable. Showing the original preview.');
    expect(container.textContent).toContain('source unknown');
    expect(container.textContent).not.toContain(savedNote.text);
    expect(container.textContent).not.toContain('Saved memory — outside current graph');
  });

  it('aborts the read when Brain unmounts and ignores a late response', async () => {
    let resolveRead!: (memory: typeof savedNote) => void;
    const read = new Promise<typeof savedNote>(resolve => { resolveRead = resolve; });
    apiFetchMock.mockImplementation((url: string) => {
      if (url.startsWith('/api/brain/memory?')) return read;
      if (url.startsWith('/api/brain/graph')) return Promise.resolve(graphPayload);
      return Promise.resolve([]);
    });
    await renderBrain();
    await focusPreview();
    const readCall = apiFetchMock.mock.calls.find(([url]) => url.startsWith('/api/brain/memory?'));
    await act(async () => { root.render(null); });
    expect(readCall?.[1]?.signal?.aborted).toBe(true);
    await act(async () => { resolveRead(savedNote); await read; });
    expect(container.textContent).toBe('');
  });

  it('does not replace a newer selection when an older saved-memory read resolves', async () => {
    let resolveOld!: (memory: typeof savedNote) => void;
    const oldRead = new Promise<typeof savedNote>(resolve => { resolveOld = resolve; });
    apiFetchMock.mockImplementation((url: string) => {
      if (url.startsWith('/api/brain/memory?')) return oldRead;
      if (url.startsWith('/api/brain/graph')) return Promise.resolve(graphPayload);
      return Promise.resolve([]);
    });
    await renderBrain();
    await focusPreview();
    expect(container.textContent).toContain('Loading saved memory…');
    const readCall = apiFetchMock.mock.calls.find(([url]) => url.startsWith('/api/brain/memory?'));
    await focusPreview('note:loaded', 'Newer preview');
    await act(async () => { resolveOld(savedNote); await oldRead; });
    expect(container.textContent).toContain('Loaded note');
    expect(container.textContent).not.toContain(savedNote.text);
    expect(container.textContent).not.toContain('Original note preview');
    expect(readCall?.[1]?.signal?.aborted).toBe(true);
  });
});
