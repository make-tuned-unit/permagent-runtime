/**
 * @vitest-environment jsdom
 *
 * The Brain graph's key.
 *
 * The graph shipped with zero interaction hints and three entity types sharing
 * one glyph — the filter row's ■ covers projects, tools and organisations
 * because the scene draws all three as the same cube. Colour is the only thing
 * that separates them, and nothing on screen said so.
 *
 * The other half is honesty. Every link in this scene carries a travelling
 * light, always, at a speed set by the link's weight. Under the Chip doctrine
 * a pulse is a claim that something is happening right now, so a scene of
 * permanent pulses has to say out loud that it is not that.
 *
 * The key belongs to the graph, so it must not follow the user into List.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

class FakeResizeObserver {
  observe() {}
  disconnect() {}
}
(globalThis as Record<string, unknown>).ResizeObserver = FakeResizeObserver;

vi.mock('./BrainScene', () => ({
  BrainScene: vi.fn().mockImplementation(() => ({
    setData: vi.fn(),
    setSearch: vi.fn(),
    setTypeFilter: vi.fn(),
    setTimeRange: vi.fn(),
    focusSearchHit: vi.fn(),
    clearSearchFocus: vi.fn(),
    resize: vi.fn(),
    dispose: vi.fn(),
  })),
}));

vi.mock('../mobius/Mobius', () => ({ Mobius: () => null }));

const graph = {
  self: { name: 'Agent', id: 'self' },
  entities: [{ id: 'e1', type: 'project', name: 'Acme', note: '' }],
  memories: [{
    id: 'm1', text: 'a memory', description: null, ent: [], age: 0.1, weight: 0.6,
    timestamp: '2026-08-01T00:00:00Z',
  }],
};

vi.mock('../../lib/api', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../../lib/api')>();
  return {
    ...actual,
    api: {
      ...actual.api,
      searchBrain: vi.fn(),
      getBrainMemories: vi.fn(() => Promise.resolve({ memories: [], total: 0, has_more: false })),
    },
    apiFetch: vi.fn(() => Promise.resolve(graph)),
  };
});

vi.mock('../world/shared/worldEvents', () => ({ subscribeWorldEvents: () => () => {} }));

import { BrainView } from './BrainView';
import { VIEW_MODE_KEY } from './viewMode';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  localStorage.clear();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  localStorage.clear();
});

async function render(mode: 'graph' | 'list') {
  localStorage.setItem(VIEW_MODE_KEY, mode);
  await act(async () => root.render(<BrainView />));
}

const legend = () => container.querySelector('[data-testid="canvas-legend"]');

describe('Brain graph key', () => {
  it('teaches the gestures the scene actually has', async () => {
    await render('graph');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('Drag');
    expect(text).toContain('Scroll');
    expect(text).toContain('Hover a node');
    expect(text).toContain('Click a node');
  });

  it('does not offer a gesture the scene does not have', async () => {
    await render('graph');
    // There is no panning in BrainScene — only yaw/pitch and wheel-zoom — and
    // no keyboard control at all, both of which the World's key does offer.
    const text = (legend()!.textContent ?? '').toLowerCase();
    expect(text).not.toContain('right-drag');
    expect(text).not.toContain('slides the view');
    expect(text).not.toContain('arrow keys');
  });

  it('says what separates the three things that share one shape', async () => {
    await render('graph');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('project');
    expect(text).toContain('tool');
    expect(text).toContain('organisation');
    expect(text).toContain('colour tells them apart');
  });

  it('admits the travelling lights are constant, not live traffic', async () => {
    await render('graph');
    const text = legend()!.textContent ?? '';
    expect(text).toContain('always on');
    expect(text).toContain('not live traffic');
  });

  it('stays on the graph — List has its own vocabulary', async () => {
    await render('list');
    expect(legend()).toBeNull();
    expect(container.querySelector('[data-testid="canvas-legend-open"]')).toBeNull();
  });
});
