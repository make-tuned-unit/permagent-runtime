/**
 * @vitest-environment jsdom
 *
 * Kanban card detail: "clicking a card in Kanban should open the card so I can
 * see the details. Right now nothing happens when I click it."
 *
 * Nothing happened because the pointer-up handler opened a modal only for
 * `cardType === 'goal'`; a standard to-do fell through the branch and there was
 * no card-detail component in the UI at all. These pin the three halves of the
 * fix that can each break independently:
 *
 *   - a click (a press that never travels) opens the detail;
 *   - closing it returns to the board with the card still there;
 *   - a DRAG (a press that travels past the 4px threshold) opens nothing —
 *     the regression an over-eager "just make click work" fix would cause.
 *
 * The board drags on raw pointer events (HTML5 DnD is eaten by Tauri's native
 * layer), and jsdom ships no PointerEvent — so the helper below dispatches
 * MouseEvents under pointer type names, which is exactly what React's
 * `onPointerDown` and the window listeners subscribe to.
 */
import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { readConfig: vi.fn(() => new Promise(() => {})) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));
vi.mock('../../lib/useGoalEvents', () => ({ useGoalEvents: () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn(() => Promise.resolve()) }));

import { ProjectKanban } from './ProjectsView';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const apiFetchMock = vi.mocked(apiFetch);

let storage: Record<string, string> = {};
vi.stubGlobal('localStorage', {
  getItem: (k: string) => (k in storage ? storage[k] : null),
  setItem: (k: string, v: string) => { storage[k] = v; },
  removeItem: (k: string) => { delete storage[k]; },
  clear: () => { storage = {}; },
});

const project = {
  id: 'p1', slug: 'p1', name: 'Kinros', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [],
  metadataJson: {}, createdAt: '', updatedAt: '', lastOpenedAt: '',
};

const column = {
  id: 'col1', projectId: 'p1', name: 'Backlog', position: 0,
  columnKind: 'manual', stateBinding: null, wipLimit: null,
};

const card = {
  id: 'c1', projectId: 'p1', cardType: 'standard',
  title: 'Renew the domain', description: 'Before it lapses',
  columnId: 'col1', position: 0, createdBy: 'user', assignedTo: null,
  metadataJson: { dueDate: '2026-09-01' },
  createdAt: '2026-08-01T10:00:00Z', updatedAt: '2026-08-02T10:00:00Z',
  archivedAt: null,
};

function serveBoard() {
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects/p1/columns') return Promise.resolve([column]);
    if (url === '/api/projects/p1/cards') return Promise.resolve([card]);
    if (url === '/api/projects/p1/cards/c1') return Promise.resolve(card);
    return Promise.resolve([]);
  }) as typeof apiFetch);
}

/** jsdom has no PointerEvent; React and the window listeners key off the NAME. */
function pointer(type: string, target: EventTarget, x: number, y: number) {
  target.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  storage = {};
  apiFetchMock.mockReset();
  serveBoard();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderBoard() {
  await act(async () => { root.render(<ProjectKanban project={project} />); });
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

/** The card on the board, found by its text rather than its aria-label — the
 *  label changes with openability, and a locator that only matches the FIXED
 *  board would turn a real regression into a confusing "not rendered". */
function cardEl(): HTMLElement {
  const el = Array.from(container.querySelectorAll('[role="button"]'))
    .find(n => n.textContent?.includes('Renew the domain'));
  if (!el) throw new Error('card not rendered');
  return el as HTMLElement;
}

/** The modal is identified by its own body, not by the title — the card title
 *  is on the board too, so matching on it would pass without a modal. */
function detailIsOpen(): boolean {
  return document.body.textContent?.includes('Before it lapses') === true
    && document.body.textContent?.includes('Edit card') === true;
}

describe('clicking a Kanban card opens its detail', () => {
  it('opens the detail view for a standard card', async () => {
    await renderBoard();
    expect(detailIsOpen()).toBe(false);

    const el = cardEl();
    await act(async () => { pointer('pointerdown', el, 40, 40); });
    await act(async () => { pointer('pointerup', el, 40, 40); });
    for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });

    expect(detailIsOpen()).toBe(true);
    // It shows the card's real detail, fetched by id.
    expect(apiFetchMock.mock.calls.map(c => String(c[0]))).toContain('/api/projects/p1/cards/c1');
    expect(document.body.textContent).toContain('2026-09-01'); // due date
    expect(document.body.textContent).toContain('Kinros'); // project
    expect(document.body.textContent).toContain('Backlog'); // column
  });

  it('closes cleanly, returning to the board', async () => {
    await renderBoard();
    const el = cardEl();
    await act(async () => { pointer('pointerdown', el, 40, 40); });
    await act(async () => { pointer('pointerup', el, 40, 40); });
    for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
    expect(detailIsOpen()).toBe(true);

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    for (let i = 0; i < 2; i++) await act(async () => { await Promise.resolve(); });

    expect(detailIsOpen()).toBe(false);
    expect(cardEl().textContent).toContain('Renew the domain');
  });

  it('discards an invalid edit without leaving its error or changing the saved card', async () => {
    await renderBoard();
    const el = cardEl();
    await act(async () => { pointer('pointerdown', el, 40, 40); });
    await act(async () => { pointer('pointerup', el, 40, 40); });

    const click = async (label: string) => {
      const button = Array.from(document.querySelectorAll('button'))
        .find(node => node.textContent?.trim() === label);
      expect(button).toBeTruthy();
      await act(async () => { button!.click(); });
    };
    await click('Edit card');
    const title = document.querySelector<HTMLInputElement>('input[aria-label="Card title"]')!;
    await act(async () => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(title, '');
      title.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await click('Save changes');
    expect(document.body.textContent).toContain('A card needs a title.');

    await click('Discard');
    expect(detailIsOpen()).toBe(true);
    expect(document.body.textContent).not.toContain('A card needs a title.');
    expect(cardEl().textContent).toContain(card.title);
    expect(apiFetchMock.mock.calls.some(([, options]) =>
      options?.method === 'PATCH' || options?.method === 'PUT')).toBe(false);

    await click('Edit card');
    expect(document.querySelector<HTMLInputElement>('input[aria-label="Card title"]')?.value).toBe(card.title);
  });

  it('names the new-card title input and cancels without creating a card', async () => {
    await renderBoard();
    const add = Array.from(container.querySelectorAll('button'))
      .find(node => node.textContent?.includes('Add card'));
    expect(add).toBeTruthy();
    await act(async () => { add!.click(); });
    const input = container.querySelector<HTMLInputElement>('input[aria-label="New card title"]');
    expect(input).not.toBeNull();
    await act(async () => {
      input!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(container.querySelector('input[aria-label="New card title"]')).toBeNull();
    expect(apiFetchMock.mock.calls.some(([, options]) => options?.method === 'POST')).toBe(false);
  });

  it('does not open the detail when the press was a drag', async () => {
    await renderBoard();
    const el = cardEl();
    await act(async () => { pointer('pointerdown', el, 40, 40); });
    // Travel well past the 4px click/drag threshold, then release.
    await act(async () => { pointer('pointermove', window, 200, 300); });
    await act(async () => { pointer('pointerup', window, 200, 300); });
    for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });

    expect(detailIsOpen()).toBe(false);
    expect(apiFetchMock.mock.calls.map(c => String(c[0]))).not.toContain('/api/projects/p1/cards/c1');
  });
});


it('hides terminal work by default and reveals it when a canonical deep link arrives', async () => {
  const history = { ...column, id: 'done', name: 'Complete', stateBinding: 'complete' };
  const completed = { ...card, id: 'done-card', title: 'A completed task', columnId: 'done' };
  apiFetchMock.mockImplementation((async (url: string) => {
    if (url.endsWith('/columns')) return [column, history];
    if (url.endsWith('/cards')) return [card, completed];
    return [];
  }) as typeof apiFetch);
  useCommandCenter.setState({ pendingCardNavigation: null });
  const original = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = vi.fn();
  try {
    await renderBoard();
    expect(container.textContent).not.toContain('A completed task');
    expect(container.textContent).toContain('History (1)');
    await act(async () => useCommandCenter.setState({ pendingCardNavigation: {projectId: 'p1', cardId: 'done-card'} }));
    expect(container.textContent).toContain('A completed task');
    const historyButton = [...container.querySelectorAll('nav button')].find(b => b.textContent === 'History (1)')!;
    expect(historyButton.getAttribute('aria-pressed')).toBe('true');
    expect(useCommandCenter.getState().pendingCardNavigation).toBeNull();
    expect(apiFetchMock.mock.calls.every(([, opts]) => !opts?.method || opts.method === 'GET')).toBe(true);
  } finally { HTMLElement.prototype.scrollIntoView = original; }
});
