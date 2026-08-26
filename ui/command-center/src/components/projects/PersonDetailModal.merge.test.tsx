/**
 * @vitest-environment jsdom
 *
 * PersonDetailModal merge wiring (duplicate-cleanup epic): the "Merge into…"
 * footer action opens MergePersonPanel inside the modal body (not a second
 * overlay); a successful merge shows the daemon's summary plus an Undo
 * button; Undo POSTs `/api/people/merges/{id}/undo`.
 *
 * MergePersonPanel itself is unit-tested separately (MergePersonPanel.test.tsx)
 * — here we only pin the wiring: opening it, receiving its report, and undo.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { MergeReport, Person, UndoReport } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const person: Person = {
  entity_uuid: 'uuid-jane',
  canonical_id: 'person:jane-doe',
  display_name: 'Jane Doe',
  role: 'CTO',
  company: 'Acme',
  email: null,
  phone: null,
  notes: null,
  last_contact_at: null,
  birthday: null,
  relationship_strength: null,
  how_met: null,
  linkedin: null,
  x_handle: null,
  facebook: null,
  instagram: null,
  personal_site: null,
  photo_url: null,
  find_online_hints: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

const mergeReport: MergeReport = {
  merge_id: 'merge-1', survivor_uuid: 'uuid-jane', survivor_name: 'Jane Doe',
  duplicate_uuid: 'uuid-jane2', duplicate_name: 'Jane D.',
  meetings_moved: 3, project_links_moved: 1, project_links_dropped: 0,
  fields_copied: 1, graph_edges_moved: 2, aliases_recorded: 1,
  summary: 'Merged Jane D. into Jane Doe.',
};

const undoReport: UndoReport = {
  merge_id: 'merge-1', restored_uuid: 'uuid-jane2', restored_name: 'Jane D.',
  meetings_restored: 3, project_links_restored: 1, graph_edges_restored: 2,
  aliases_removed: 1, not_reverted: [],
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockImplementation((url: string, opts?: RequestInit) => {
    if (url.endsWith('/relationships') || url.endsWith('/activity') || url.endsWith('/meetings') || url === '/api/people') return Promise.resolve([]);
    if (url.endsWith('/projects')) return Promise.resolve([]);
    if (url === '/api/people/directory') return Promise.resolve([]);
    if (url === '/api/people/duplicates?limit=50') return Promise.resolve([]);
    if (url === '/api/people/merges/merge-1/undo' && opts?.method === 'POST') return Promise.resolve(undoReport);
    return Promise.resolve(undefined);
  });
  useCommandCenter.setState({ peopleRev: 0 });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => root.render(
    <PersonDetailModal projectId={null} person={person} onClose={() => {}} />,
  ));
}

function buttonMatching(pattern: string): HTMLButtonElement {
  const btns = [...container.querySelectorAll('button')];
  const btn = btns.find(b => b.textContent?.includes(pattern));
  if (!btn) throw new Error(`no button containing "${pattern}" — have: ${btns.map(b => b.textContent).join(' | ')}`);
  return btn as HTMLButtonElement;
}

async function click(btn: HTMLButtonElement) {
  await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

describe('PersonDetailModal — Merge into…', () => {
  it('opens MergePersonPanel inside the modal body', async () => {
    await render();
    await click(buttonMatching('Merge into…'));
    // The panel's own pick-step copy proves it mounted in the body.
    expect(container.textContent).toContain('Likely duplicates');
    expect(container.textContent).toContain('Or search the directory');
  });

  /** Wires apiFetch for a full pick → preview → confirm merge drive, with one
   *  directory candidate ("Jane D.") and no duplicate suggestions. */
  function mockMergeableDirectory() {
    apiFetch.mockImplementation((url: string, opts?: RequestInit) => {
      if (url === '/api/people/directory') {
        return Promise.resolve([{ ...person, entity_uuid: 'uuid-jane2', display_name: 'Jane D.', projects: [] }]);
      }
      if (url === '/api/people/duplicates?limit=50') return Promise.resolve([]);
      if (url === '/api/people/uuid-jane/merge-preview?duplicate_id=uuid-jane2') {
        return Promise.resolve({
          survivor: person, duplicate: person,
          meetings: 3, open_follow_ups: 0, project_links: [], fields: [],
          fields_kept_from_survivor: [], aliases: [], graph_edges: 0, retained: [],
        });
      }
      if (url === '/api/people/uuid-jane/merge' && opts?.method === 'POST') return Promise.resolve(mergeReport);
      if (url === '/api/people/merges/merge-1/undo' && opts?.method === 'POST') return Promise.resolve(undoReport);
      if (url.endsWith('/relationships') || url.endsWith('/activity') || url.endsWith('/meetings') || url === '/api/people') return Promise.resolve([]);
      if (url.endsWith('/projects')) return Promise.resolve([]);
      return Promise.resolve(undefined);
    });
  }

  it('a successful merge shows the summary and an Undo button', async () => {
    mockMergeableDirectory();
    await render();
    await click(buttonMatching('Merge into…'));
    await click(buttonMatching('Jane D.'));
    await click(buttonMatching('Merge and delete Jane D.'));

    expect(container.textContent).toContain('Merged Jane D. into Jane Doe.');
    expect(container.textContent).toContain('Undo merge');
    expect(useCommandCenter.getState().peopleRev).toBe(1);
  });

  it('Undo POSTs /api/people/merges/{id}/undo', async () => {
    mockMergeableDirectory();
    await render();
    await click(buttonMatching('Merge into…'));
    await click(buttonMatching('Jane D.'));
    await click(buttonMatching('Merge and delete Jane D.'));
    await click(buttonMatching('Undo merge'));

    const undoCall = apiFetch.mock.calls.find(([url, opts]) => url === '/api/people/merges/merge-1/undo' && (opts as RequestInit | undefined)?.method === 'POST');
    expect(undoCall).toBeTruthy();
    expect(container.textContent).toContain('restored Jane D.');
  });
});
