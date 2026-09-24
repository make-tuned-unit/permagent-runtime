/**
 * @vitest-environment jsdom
 *
 * The Grow header's "N goals · N people" line (QA 2026-09-24).
 *
 * Evntally's board held eight goal cards and the Actions tab reported goals in
 * flight, yet the header read "0 goals". GrowView fetched every card and
 * filtered on `c.card_type`, but CardResponse serialises camelCase
 * (`cardType`), so the filter matched nothing. It now asks the daemon for goal
 * cards (`?card_type=goal`) — the same `cards::list_cards` read the growth
 * inbox ranks — so both surfaces count one set of cards.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {},
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { GrowView } from './GrowView';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);

const project = {
  id: 'evn', slug: 'evntally', name: 'Evntally', description: '', status: 'active',
  rootPath: null, siteUrl: null, repoUrl: null, tags: [], metadataJson: {},
  createdAt: '', updatedAt: '', lastOpenedAt: '',
};

/** The real wire shape: CardResponse is `rename_all = "camelCase"`. */
const card = (id: string, cardType: string) => ({
  id, projectId: 'evn', cardType, title: id, description: '', columnId: 'c',
  position: 0, createdBy: 'user', assignedTo: null, metadataJson: {},
  createdAt: '', updatedAt: '', archivedAt: null,
});

const goals = ['g1', 'g2', 'g3'].map((id) => card(id, 'goal'));
const everything = [...goals, card('s1', 'standard'), card('p1', 'social_post')];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetchMock.mockReset();
  apiFetchMock.mockImplementation(((url: string) => {
    if (url === '/api/projects') return Promise.resolve([project]);
    if (url.includes('/cards?card_type=goal')) return Promise.resolve(goals);
    if (url.includes('/cards?card_type=')) return Promise.resolve([]);
    if (url.endsWith('/cards')) return Promise.resolve(everything);
    if (url.endsWith('/people')) return Promise.resolve([{ id: 'x' }]);
    return Promise.resolve([]);
  }) as typeof apiFetch);
  useCommandCenter.setState({ openGrowForProject: null, workspaces: [], currentProjectId: 'evn' });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('Grow header context counts', () => {
  it('counts the goal cards on the board, singular/plural correct', async () => {
    await act(async () => { root.render(<GrowView />); });
    await act(async () => { await Promise.resolve(); });
    await act(async () => { await new Promise((r) => setTimeout(r, 0)); });

    expect(container.textContent).toContain('3 goals · 1 person');
    expect(container.textContent).not.toContain('0 goals');
  });
});
