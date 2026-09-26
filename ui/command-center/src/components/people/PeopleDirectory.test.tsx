/** @vitest-environment jsdom */
/**
 * The directory's job is to make the invisible cohort visible. These tests pin
 * the behaviours that decide whether it actually does that — the "no project"
 * affordance, the honest Brain-down state, and the duplicate chip's deliberately
 * narrow rule (the two richer rules were dropped because they misfired on the
 * real corpus: shared-surname flags two unrelated Dixons, and email-domain can
 * never fire at all since the graph overlay nulls the column).
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PeopleDirectory } from './PeopleDirectory';
import { useCommandCenter } from '../../lib/store';
import type { DirectoryPerson } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const base = {
  canonical_id: '',
  role: null,
  company: null,
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
  created_at: '',
  updated_at: '',
};

function person(
  entity_uuid: string,
  display_name: string,
  extra: Partial<DirectoryPerson> = {},
): DirectoryPerson {
  return { ...base, entity_uuid, display_name, projects: [], ...extra };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('PeopleDirectory', () => {
  it('surfaces people with no project — the cohort no other UI can reach', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ashley Lecroy'),
      person('b', 'Craig Beaton', {
        role: 'Advisor',
        projects: [{ project_id: 'p1', project_name: 'Harbourview' }],
      }),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).toContain('Ashley Lecroy');
    expect(container.textContent).toContain('Craig Beaton');
    // The chip for the associated one, and the honest marker for the other.
    expect(container.textContent).toContain('Harbourview');
    expect(container.textContent).toContain('no project');
    // The count is the readable proof this surface exists for.
    expect(container.textContent).toContain('1 in no project');
  });

  it('shows last-contact decay and a due follow-up', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ada Lovelace', {
        last_contact_at: '2026-01-01T00:00:00Z',
        next_follow_up_at: '2020-01-01T00:00:00Z',
      }),
    ]);
    await act(async () => root.render(<PeopleDirectory />));
    expect(container.textContent).toContain('follow up');
    expect(container.textContent).toMatch(/mo ago|never/);
  });

  it('flags a name that is a strict prefix of another, and nothing else', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ada'),
      person('b', 'Ada Lovelace'),
      // Same surname, genuinely different people — must NOT be flagged.
      person('c', 'Leanne Dixon'),
      person('d', 'Liam Dixon'),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    const chips = container.textContent?.match(/possible duplicate/g) ?? [];
    expect(chips).toHaveLength(2);
  });

  it('says so when rows came back but the Brain could not fill attributes', async () => {
    apiFetch.mockResolvedValue([person('a', 'Ashley Lecroy'), person('b', 'Craig Beaton')]);
    await act(async () => root.render(<PeopleDirectory />));

    // A list of bare names is otherwise indistinguishable from data loss.
    expect(container.textContent).toContain("the Brain isn't available");
  });

  it('does not claim the Brain is down when attributes are present', async () => {
    apiFetch.mockResolvedValue([person('a', 'Ashley Lecroy', { company: 'Harbourview RA' })]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).not.toContain("the Brain isn't available");
    expect(container.textContent).toContain('Harbourview RA');
  });

  it('distinguishes a failed load from an empty directory', async () => {
    apiFetch.mockRejectedValue(new Error('boom'));
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).toContain("Couldn't load people.");
    // StateBlock's shared retry affordance (R7: converged onto the same
    // component Projects/Dashboard use, instead of a fourth hand-rolled label).
    expect(container.textContent).toContain('Try again');
    expect(container.textContent).not.toContain('No people yet.');
  });

  it('opens an existing person rather than reporting a false success', async () => {
    apiFetch.mockImplementation((_url: string, init?: { method?: string }) => {
      if (init?.method === 'POST') {
        return Promise.resolve({ person: person('a', 'Ada Lovelace'), created: false });
      }
      return Promise.resolve([person('a', 'Ada Lovelace')]);
    });
    await act(async () => root.render(<PeopleDirectory />));

    const addToggle = Array.from(container.querySelectorAll('button')).find(b =>
      b.textContent?.includes('Add person'),
    )!;
    await act(async () => addToggle.click());

    const input = container.querySelector('input[placeholder="Full name"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      'value',
    )!.set!;
    await act(async () => {
      setter.call(input, 'Ada Lovelace');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const addBtn = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent === 'Add',
    )!;
    await act(async () => addBtn.click());

    expect(container.textContent).toContain('already exists');
  });

  it('sends the create body in snake_case', async () => {
    apiFetch.mockImplementation((_url: string, init?: { method?: string }) => {
      if (init?.method === 'POST') {
        return Promise.resolve({ person: person('n', 'New Person'), created: true });
      }
      return Promise.resolve([]);
    });
    await act(async () => root.render(<PeopleDirectory />));

    const addToggle = Array.from(container.querySelectorAll('button')).find(b =>
      b.textContent?.includes('Add person'),
    )!;
    await act(async () => addToggle.click());
    const input = container.querySelector('input[placeholder="Full name"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(
      window.HTMLInputElement.prototype,
      'value',
    )!.set!;
    await act(async () => {
      setter.call(input, 'New Person');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const addBtn = Array.from(container.querySelectorAll('button')).find(
      b => b.textContent === 'Add',
    )!;
    await act(async () => addBtn.click());

    const post = apiFetch.mock.calls.find(c => c[1]?.method === 'POST')!;
    expect(post[0]).toBe('/api/people');
    // The People endpoints carry no serde rename_all — a camelCase key would
    // fail to deserialize server-side rather than binding an empty name.
    expect(JSON.parse(post[1].body)).toEqual({ display_name: 'New Person' });
  });

  // P2 — QA 2026-09-24: 36 identical "never" rows said nothing a single
  // banner doesn't, on a directory where no daemon has ever logged a meeting.
  it('shows one banner instead of a "never" on every row when nothing has an interaction', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ashley Lecroy'),
      person('b', 'Craig Beaton'),
      person('c', 'Dana Okafor'),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).toContain('No interactions recorded yet.');
    expect(container.textContent).not.toContain('never');
  });

  it('does not show the banner once at least one contact has a real interaction', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ashley Lecroy'),
      person('b', 'Craig Beaton', { last_contact_at: '2026-01-01T00:00:00Z' }),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).not.toContain('No interactions recorded yet.');
    expect(container.textContent).toContain('never');
  });

  // P3 — QA 2026-09-24: the backend is adding `kind`; the directory must
  // filter organisations by it and show the user's own record as "You".
  it('shows an Organisations filter chip and hides it when the backend sends no kind', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ashley Lecroy', { kind: 'person' }),
      person('org1', "Harbourview Residents' Association", { kind: 'organisation' }),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    const chip = Array.from(container.querySelectorAll('button')).find(b =>
      b.textContent?.includes('Organisations'));
    expect(chip).toBeTruthy();
    await act(async () => chip!.click());
    expect(container.textContent).toContain("Harbourview Residents' Association");
    expect(container.textContent).not.toContain('Ashley Lecroy');
  });

  // R2 (UX_STANDARD): a filter matching nobody must say so, not render an
  // empty scroll area that looks identical to a slow load or a broken filter.
  it('says so when the Organisations filter matches nobody visible', async () => {
    apiFetch.mockResolvedValue([person('a', 'Ashley Lecroy', { kind: 'person' })]);
    await act(async () => root.render(<PeopleDirectory />));
    // No organisation in this directory — the chip itself must not even show
    // (covered above) — this test is for a cohort that legitimately filters
    // to zero, e.g. "Follow-up" with nothing due.
    const followUpChip = Array.from(container.querySelectorAll('button')).find(b =>
      b.textContent?.startsWith('Follow-up'))!;
    await act(async () => followUpChip.click());
    expect(container.textContent).toContain('No one matches this filter.');
  });

  it('shows no Organisations chip on an older daemon with no kind field', async () => {
    apiFetch.mockResolvedValue([person('a', 'Ashley Lecroy')]);
    await act(async () => root.render(<PeopleDirectory />));

    const chip = Array.from(container.querySelectorAll('button')).find(b =>
      b.textContent?.includes('Organisations'));
    expect(chip).toBeFalsy();
  });

  it('marks organisation and person rows distinctly, reusing the Brain graph\'s own round/square convention', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Ashley Lecroy', { kind: 'person' }),
      person('org1', "Harbourview Residents' Association", { kind: 'organisation' }),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.querySelector('[data-testid="people-row-person"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="people-row-organisation"]')).toBeTruthy();
  });

  it('shows the user\'s own record as "You", not their name, and never flags it a duplicate', async () => {
    apiFetch.mockResolvedValue([
      person('me', 'Sam Okafor', { kind: 'self' }),
      person('b', 'Sam', {}),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    expect(container.textContent).toContain('You');
    expect(container.textContent).not.toContain('Sam Okafor');
    expect(container.textContent).not.toContain('possible duplicate');
  });

  it('gives only the "You" row an Edit-in-Settings affordance, which opens Settings → Preferences', async () => {
    apiFetch.mockResolvedValue([
      person('me', 'Sam Okafor', { kind: 'self' }),
      person('b', 'Priya Shah', {}),
    ]);
    await act(async () => root.render(<PeopleDirectory />));

    const editButtons = container.querySelectorAll('[data-testid="people-row-self-edit-in-settings"]');
    expect(editButtons.length).toBe(1);

    useCommandCenter.setState({ activePanel: 'chat', pendingSettingsSection: null });
    await act(async () => {
      editButtons[0].dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    const state = useCommandCenter.getState();
    expect(state.activePanel).toBe('settings');
    expect(state.pendingSettingsSection).toBe('preferences');
  });
});
