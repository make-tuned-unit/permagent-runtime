/**
 * @vitest-environment jsdom
 *
 * PersonDetailModal delete wiring: a real two-step confirm (naming the person
 * and listing counts already loaded in this modal — never invented), then
 * `DELETE /api/people/{id}` with `{ confirm: true }`, then the daemon's
 * `retained` strings shown verbatim.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { DeleteReport, Person, PersonMeeting, PersonProject } from './types';

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

const meetings: PersonMeeting[] = [
  {
    id: 'm1', entity_uuid: 'uuid-jane', title: 'Coffee', starts_at: '2026-02-01T10:00:00Z',
    ends_at: null, notes: '', calendar_synced: false, project_id: null,
    follow_up_at: null, follow_up_note: '', follow_up_done: false, calendar_uid: null,
    created_at: '2026-02-01T10:00:00Z', updated_at: '2026-02-01T10:00:00Z',
  },
  {
    id: 'm2', entity_uuid: 'uuid-jane', title: 'Follow-up', starts_at: '2026-02-05T10:00:00Z',
    ends_at: null, notes: '', calendar_synced: false, project_id: null,
    follow_up_at: null, follow_up_note: '', follow_up_done: false, calendar_uid: null,
    created_at: '2026-02-05T10:00:00Z', updated_at: '2026-02-05T10:00:00Z',
  },
];

const projects: PersonProject[] = [
  { project_id: 'p1', project_name: 'Acme Deal', project_status: 'active', role: 'Advisor', added_at: '2026-01-01T00:00:00Z' },
];

const deleteReport: DeleteReport = {
  entity_uuid: 'uuid-jane', display_name: 'Jane Doe', log_id: 'log-1',
  meetings_deleted: 2, project_links_deleted: 1, graph_edges_deleted: 0, aliases_deleted: 0,
  retained: ['Memories mentioning Jane Doe stay in the Brain.'],
};

let container: HTMLDivElement;
let root: Root;
let onCloseSpy: ReturnType<typeof vi.fn>;

beforeEach(() => {
  onCloseSpy = vi.fn();
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url.endsWith('/meetings')) return Promise.resolve(meetings);
    if (url.endsWith('/relationships') || url.endsWith('/activity') || url === '/api/people') return Promise.resolve([]);
    if (url.endsWith('/projects')) return Promise.resolve(projects);
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
    <PersonDetailModal projectId={null} person={person} onClose={onCloseSpy} />,
  ));
}

function buttonMatching(pattern: string): HTMLButtonElement {
  const btns = [...container.querySelectorAll('button')];
  const btn = btns.find(b => b.textContent?.includes(pattern));
  if (!btn) throw new Error(`no button containing "${pattern}" — have: ${btns.map(b => b.textContent).join(' | ')}`);
  return btn as HTMLButtonElement;
}

function findButton(pattern: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll('button')].find(b => b.textContent?.includes(pattern));
}

async function click(btn: HTMLButtonElement) {
  await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

describe('PersonDetailModal — delete confirmation gate', () => {
  it('does NOT delete on the first click — it shows a named, itemized confirm', async () => {
    await render();
    await click(buttonMatching('Delete person'));

    expect(apiFetch.mock.calls.some(([, opts]) => (opts as RequestInit | undefined)?.method === 'DELETE')).toBe(false);
    expect(container.textContent).toContain('Delete Jane Doe?');
    // Counts come from data this modal already loaded — not invented.
    expect(container.textContent).toContain('2 logged meetings');
    expect(container.textContent).toContain('1 project link');
  });

  it('Keep cancels back to the normal view without deleting', async () => {
    await render();
    await click(buttonMatching('Delete person'));
    await click(buttonMatching('Keep'));

    expect(findButton('Delete person')).toBeTruthy();
    expect(container.textContent).not.toContain('Delete Jane Doe?');
    expect(apiFetch.mock.calls.some(([, opts]) => (opts as RequestInit | undefined)?.method === 'DELETE')).toBe(false);
  });
});

describe('PersonDetailModal — confirmed delete', () => {
  it('sends DELETE with confirm:true and shows the retained strings', async () => {
    apiFetch.mockImplementation((url: string, opts?: RequestInit) => {
      if (url.endsWith('/meetings')) return Promise.resolve(meetings);
      if (url.endsWith('/relationships') || url.endsWith('/activity') || url === '/api/people') return Promise.resolve([]);
      if (url.endsWith('/projects')) return Promise.resolve(projects);
      if (url === '/api/people/uuid-jane' && opts?.method === 'DELETE') return Promise.resolve(deleteReport);
      return Promise.resolve(undefined);
    });
    await render();
    await click(buttonMatching('Delete person'));
    await click(buttonMatching('Confirm delete Jane Doe'));

    const deleteCall = apiFetch.mock.calls.find(([url, opts]) => url === '/api/people/uuid-jane' && (opts as RequestInit | undefined)?.method === 'DELETE');
    expect(deleteCall).toBeTruthy();
    const body = JSON.parse((deleteCall![1] as RequestInit).body as string);
    expect(body).toEqual({ confirm: true });

    expect(useCommandCenter.getState().peopleRev).toBe(1);
    expect(container.textContent).toContain('Memories mentioning Jane Doe stay in the Brain.');
    // The profile fields are gone — this is a terminal state, not editable.
    expect(findButton('Delete person')).toBeFalsy();
  });

  it('closes only when the user dismisses the deleted card', async () => {
    apiFetch.mockImplementation((url: string, opts?: RequestInit) => {
      if (url.endsWith('/meetings')) return Promise.resolve(meetings);
      if (url.endsWith('/relationships') || url.endsWith('/activity') || url === '/api/people') return Promise.resolve([]);
      if (url.endsWith('/projects')) return Promise.resolve(projects);
      if (url === '/api/people/uuid-jane' && opts?.method === 'DELETE') return Promise.resolve(deleteReport);
      return Promise.resolve(undefined);
    });
    await render();
    await click(buttonMatching('Delete person'));
    await click(buttonMatching('Confirm delete Jane Doe'));
    expect(onCloseSpy).not.toHaveBeenCalled();

    await click(buttonMatching('Close'));
    expect(onCloseSpy).toHaveBeenCalledTimes(1);
  });
});
