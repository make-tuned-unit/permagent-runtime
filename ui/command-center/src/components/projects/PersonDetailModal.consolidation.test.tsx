/**
 * @vitest-environment jsdom
 *
 * The person panel used to be its own modal.
 *
 * `PersonDetailShell` was 108 lines that re-implemented `common/DetailModal`:
 * a header with a title, a badge and a close button; a scrollable body; a
 * footer; an Escape handler; and a hand-rolled `backdrop-filter` drawer
 * surface. R12 deleted it and made the panel a caller of the shared shell
 * instead — one modal concept, one place.
 *
 * A consolidation is exactly the kind of change whose risk is invisible: the
 * eight existing PersonDetailModal test files each cover one section deeply,
 * and every one of them would still pass if the rebuild quietly dropped a
 * DIFFERENT section. So this file is the inventory, taken before the rewrite
 * and asserted after it: every section of the body, every footer action in
 * every footer state, and the shell behaviours the drawer had.
 *
 * It is deliberately shallow — it asks "is this still reachable", not "does it
 * work". What each one does is already pinned by its own file.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { Person, PersonActivity, PersonMeeting, PersonProject, PersonRelationship } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const person: Person = {
  entity_uuid: 'uuid-jane',
  canonical_id: 'person:jane-doe',
  display_name: 'Jane Doe',
  role: 'Head of Ops', company: 'Acme', email: 'jane@acme.example', phone: null, notes: null,
  last_contact_at: null, birthday: null, relationship_strength: null, how_met: null,
  linkedin: null, x_handle: null, facebook: null, instagram: null, personal_site: null,
  photo_url: null, find_online_hints: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
};

const meetings: PersonMeeting[] = [{
  id: 'm1', entity_uuid: 'uuid-jane', title: 'Coffee', starts_at: '2026-02-01T10:00:00Z',
  ends_at: null, notes: '', calendar_synced: false, project_id: null,
  follow_up_at: null, follow_up_note: '', follow_up_done: false, calendar_uid: null,
  created_at: '2026-02-01T10:00:00Z', updated_at: '2026-02-01T10:00:00Z',
}];

const projects: PersonProject[] = [
  { project_id: 'p1', project_name: 'Acme Deal', project_status: 'active', role: 'Advisor', added_at: '2026-01-01T00:00:00Z' },
];

const relationships = [{
  from_entity_uuid: 'uuid-jane', to_entity_uuid: 'uuid-ken', predicate: 'works_with',
  other_person: { ...person, entity_uuid: 'uuid-ken', display_name: 'Ken Adeyemi' },
}] as unknown as PersonRelationship[];

const activity = [
  { id: 'a1', kind: 'note', title: 'Pricing note', detail: 'Wants annual billing', timestamp: '2026-02-02T10:00:00Z' },
] as unknown as PersonActivity[];

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useCommandCenter.setState({ peopleRev: 0 });
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url.endsWith('/meetings')) return Promise.resolve(meetings);
    if (url.endsWith('/relationships')) return Promise.resolve(relationships);
    if (url.endsWith('/activity')) return Promise.resolve(activity);
    if (url === '/api/people') return Promise.resolve([]);
    if (url.endsWith('/projects')) return Promise.resolve(projects);
    return Promise.resolve(undefined);
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

/** `projectId` set is the richer of the two scopes: it is the only one that
 *  renders the association line and the Remove-from-project action. */
async function render(opts: { projectId?: string | null; variant?: 'inline' | 'overlay' } = {}) {
  await act(async () => root.render(
    <PersonDetailModal
      projectId={opts.projectId === undefined ? 'p1' : opts.projectId}
      person={person}
      association={{ project_role: 'Advisor', associated_at: '2026-01-01T00:00:00Z' } as never}
      variant={opts.variant ?? 'overlay'}
      onClose={() => {}}
    />,
  ));
  for (let i = 0; i < 4; i++) await act(async () => { await Promise.resolve(); });
}

const text = () => container.textContent ?? '';
const buttons = () => [...container.querySelectorAll('button')].map(b => b.textContent ?? '');
const hasButton = (label: string) => buttons().some(t => t.includes(label));
async function click(label: string) {
  const btn = [...container.querySelectorAll('button')].find(b => b.textContent?.includes(label));
  if (!btn) throw new Error(`no button "${label}" — have: ${buttons().join(' | ')}`);
  await act(async () => { btn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  for (let i = 0; i < 3; i++) await act(async () => { await Promise.resolve(); });
}

describe('the person panel is a DetailModal, not a second modal', () => {
  it('gets the shared shell: one dialog, labelled by its own title', async () => {
    await render();
    const dialogs = container.querySelectorAll('[role="dialog"]');
    expect(dialogs.length, 'exactly one modal shell, not a shell inside a shell').toBe(1);
    const dialog = dialogs[0];
    const labelId = dialog.getAttribute('aria-labelledby');
    expect(labelId, 'the shared shell labels its dialog').toBeTruthy();
    // `useId` ids contain ':' — not a valid bare CSS selector, and jsdom ships
    // no `CSS.escape` — so the element is found by attribute instead.
    const label = [...container.querySelectorAll('[id]')].find(el => el.id === labelId);
    expect(label?.textContent).toBe('Jane Doe');
  });

  it('is a DOCK and not a modal: no scrim, no aria-modal, and Tab can leave', async () => {
    await render();
    const dialog = container.querySelector('[role="dialog"]')!;
    // `aria-modal` on a pane the user can Tab out of, over a page they can
    // still see and click, would simply be false.
    expect(dialog.getAttribute('aria-modal')).toBeNull();
    // The drawer never had a scrim; gaining one would black out the board (or
    // the People graph) this panel opens beside.
    const scrims = [...container.querySelectorAll('div')]
      .filter(d => d.style.position === 'fixed' && d.style.inset === '0px');
    expect(scrims.length, 'a dock has no scrim').toBe(0);
  });

  it('still closes on Escape, the one shell behaviour the drawer had', async () => {
    const onClose = vi.fn();
    await act(async () => root.render(
      <PersonDetailModal projectId={null} person={person} onClose={onClose} />,
    ));
    for (let i = 0; i < 3; i++) await act(async () => { await Promise.resolve(); });
    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    expect(onClose).toHaveBeenCalled();
  });

  it('keeps both placements — and the test id PeopleView locates the panel by', async () => {
    await render({ variant: 'overlay' });
    const overlay = container.querySelector<HTMLElement>('[data-testid="person-detail-panel"]')!;
    expect(overlay.style.position, 'the overlay dock pins to the window edge').toBe('fixed');
    expect(overlay.style.right).toBe('0px');

    await render({ variant: 'inline' });
    const inline = container.querySelector<HTMLElement>('[data-testid="person-detail-panel"]')!;
    expect(inline.style.position, 'the inline dock sits in PeopleView’s layout').not.toBe('fixed');
    expect(inline.style.width).toBe('400px');
  });

  it('carries no hand-rolled glass — a modal body is content, and content is opaque', async () => {
    await render();
    const glassy = [...container.querySelectorAll<HTMLElement>('div')]
      .filter(d => d.style.backdropFilter || d.style.getPropertyValue('-webkit-backdrop-filter'));
    expect(glassy.map(d => d.style.backdropFilter)).toEqual([]);
  });
});

describe('every body section survived the rebuild', () => {
  it('renders all nine of them, in the order the drawer had', async () => {
    await render();
    // B5 the edit form (all sixteen fields, each a labelled control)
    const labels = [...container.querySelectorAll('label')].map(l => l.textContent ?? '');
    for (const field of [
      'Photo', 'Role', 'Company', 'Email', 'Phone', 'LinkedIn', 'X', 'Facebook',
      'Instagram', 'Site', 'Birthday', 'Last contact', 'Relationship', 'How met',
      'Find online', 'Notes',
    ]) {
      expect(labels.some(l => l.startsWith(field)), `field "${field}"`).toBe(true);
    }
    // B6 the association line, B7-B10 the four loaded sections
    expect(text(), 'association line').toContain('Advisor');
    expect(container.querySelector('[data-testid="person-projects"]'), 'B7 projects').toBeTruthy();
    expect(container.querySelector('[data-testid="person-projects-cluster"]'), 'B7 graph cluster').toBeTruthy();
    expect(text()).toContain('Related people');   // B8
    expect(text()).toContain('Ken Adeyemi');
    expect(text()).toContain('Meetings');         // B9
    expect(text()).toContain('Coffee');
    expect(text()).toContain('Recent activity');  // B10
    expect(text()).toContain('Pricing note');
  });

  it('drops the project-scoped rows when opened from the directory', async () => {
    await render({ projectId: null });
    expect(hasButton('Remove from project'), 'no project scope, no disassociate').toBe(false);
    // …and keeps everything that is not about a project.
    expect(text()).toContain('Meetings');
    expect(hasButton('Delete person')).toBe(true);
  });
});

describe('every footer action survived the rebuild', () => {
  it('offers the six default actions', async () => {
    await render();
    for (const action of [
      'Run enrichment', 'Save', 'Merge into', 'Remove from project', 'Delete person',
    ]) {
      expect(hasButton(action), `footer action "${action}"`).toBe(true);
    }
  });

  it('swaps to the disassociate confirm, and back', async () => {
    await render();
    await click('Remove from project');
    expect(text()).toContain('Remove Jane Doe from this project?');
    expect(hasButton('Keep')).toBe(true);
    expect(hasButton('Confirm remove')).toBe(true);
    await click('Keep');
    expect(hasButton('Run enrichment'), 'back to the default footer').toBe(true);
  });

  it('swaps to the delete confirmation, which quotes the counts it loaded', async () => {
    await render();
    await click('Delete person');
    expect(text()).toContain('Confirm below to delete Jane Doe.');
    const counts = container.querySelector('[data-testid="delete-warning-counts"]')!.textContent ?? '';
    expect(counts).toContain('1 logged meeting');
    expect(counts).toContain('1 project link');
    expect(hasButton('Confirm delete Jane Doe')).toBe(true);
  });

  it('hands the whole panel to the merge flow, footer included', async () => {
    await render();
    await click('Merge into');
    // The merge panel takes over the body, and the footer is suppressed rather
    // than left offering Save on a record mid-merge.
    expect(hasButton('Run enrichment')).toBe(false);
    expect(hasButton('Delete person')).toBe(false);
  });
});
