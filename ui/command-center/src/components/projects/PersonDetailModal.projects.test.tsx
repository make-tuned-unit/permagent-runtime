/**
 * @vitest-environment jsdom
 *
 * The person modal runs four loaders. Three of them track loading/ready/error;
 * the fourth — the project list — swallowed its failure and set the list to
 * empty, which is the defect this file pins closed.
 *
 * It matters more here than "one more untracked fetch" suggests. That list's
 * length is one of the two counts the delete confirmation quotes back at the
 * user ("this deletes N logged meetings and M project links"). The sentence
 * exists so the numbers in it are real. A silently emptied list turns it into
 * "0 project links" — which is not a blank, it is a claim, made in the one
 * dialog whose whole job is to be trusted before something irreversible.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { Person, PersonMeeting, PersonProject } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const person: Person = {
  entity_uuid: 'uuid-jane',
  canonical_id: 'person:jane-doe',
  display_name: 'Jane Doe',
  role: null, company: null, email: null, phone: null, notes: null,
  last_contact_at: null, birthday: null, relationship_strength: null, how_met: null,
  linkedin: null, x_handle: null, facebook: null, instagram: null, personal_site: null,
  photo_url: null, find_online_hints: null,
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
];

const projects: PersonProject[] = [
  { project_id: 'p1', project_name: 'Acme Deal', project_status: 'active', role: 'Advisor', added_at: '2026-01-01T00:00:00Z' },
];

let container: HTMLDivElement;
let root: Root;

function mockApi({ projectsFail }: { projectsFail: boolean }) {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url.endsWith('/meetings')) return Promise.resolve(meetings);
    if (url.endsWith('/relationships') || url.endsWith('/activity') || url === '/api/people') {
      return Promise.resolve([]);
    }
    if (url.endsWith('/projects')) {
      return projectsFail ? Promise.reject(new Error('daemon down')) : Promise.resolve(projects);
    }
    return Promise.resolve(undefined);
  });
}

beforeEach(() => {
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

function button(pattern: string): HTMLButtonElement {
  const btns = [...container.querySelectorAll('button')];
  const btn = btns.find(b => b.textContent?.includes(pattern));
  if (!btn) throw new Error(`no button containing "${pattern}"`);
  return btn as HTMLButtonElement;
}

async function openDeleteConfirm() {
  await act(async () => { button('Delete').dispatchEvent(new MouseEvent('click', { bubbles: true })); });
}

describe('PersonDetailModal projects row', () => {
  it('renders the projects the person is on', async () => {
    mockApi({ projectsFail: false });
    await render();
    const chip = container.querySelector('[data-testid="person-project-p1"]');
    expect(chip).not.toBeNull();
    expect(chip!.textContent).toContain('Acme Deal');
  });

  it('navigates to the project and gets out of the way', async () => {
    mockApi({ projectsFail: false });
    const onClose = vi.fn();
    await act(async () => root.render(
      <PersonDetailModal projectId={null} person={person} onClose={onClose} />,
    ));
    const chip = container.querySelector<HTMLButtonElement>('[data-testid="person-project-p1"]')!;
    await act(async () => { chip.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    expect(useCommandCenter.getState().pendingProjectNavigation).toBe('p1');
    expect(onClose).toHaveBeenCalled();
  });

  it('is a destination, not a toggle', async () => {
    mockApi({ projectsFail: false });
    await render();
    const chip = container.querySelector('[data-testid="person-project-p1"]')!;
    // A chip that goes somewhere has no on/off state to report.
    expect(chip.getAttribute('aria-pressed')).toBeNull();
  });

  it('says a failed load failed instead of "no projects"', async () => {
    mockApi({ projectsFail: true });
    await render();
    const section = container.querySelector('[data-testid="person-projects"]')!;
    expect(section.textContent).toContain("Couldn't load");
    expect(section.textContent).not.toContain('Not on any project');
  });
});

describe('PersonDetailModal carries the graph’s premise', () => {
  it('says which group the graph puts a single-project person in', async () => {
    mockApi({ projectsFail: false });
    await render();
    const cluster = container.querySelector('[data-testid="person-projects-cluster"]')!;
    expect(cluster.textContent).toContain('Acme Deal');
  });

  it('names a multi-project person as a bridge, the way the key does', async () => {
    apiFetch.mockReset().mockImplementation((url: string) => {
      if (url.endsWith('/meetings')) return Promise.resolve(meetings);
      if (url.endsWith('/projects')) {
        return Promise.resolve([
          ...projects,
          { project_id: 'p2', project_name: 'Beta Launch', project_status: 'active', role: null, added_at: '2026-01-02T00:00:00Z' },
        ]);
      }
      return Promise.resolve([]);
    });
    await render();
    const cluster = container.querySelector('[data-testid="person-projects-cluster"]')!;
    expect(cluster.textContent).toContain('bridge');
    expect(cluster.textContent).toContain('2');
  });

  it('claims nothing about the graph when the list did not load', async () => {
    mockApi({ projectsFail: true });
    await render();
    expect(container.querySelector('[data-testid="person-projects-cluster"]')).toBeNull();
  });
});

describe('PersonDetailModal project loading', () => {
  it('counts the real project links when the list loaded', async () => {
    mockApi({ projectsFail: false });
    await render();
    await openDeleteConfirm();
    const text = container.querySelector('[data-testid="delete-warning-counts"]')!.textContent ?? '';
    expect(text).toContain('1 project link');
  });

  it('never reports a failed project load as zero links', async () => {
    mockApi({ projectsFail: true });
    await render();
    await openDeleteConfirm();
    const text = container.querySelector('[data-testid="delete-warning-counts"]')!.textContent ?? '';
    expect(text).not.toContain('0 project links');
    expect(text).toContain("didn't load");
  });
});
