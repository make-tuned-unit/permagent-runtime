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

  it('flags a name that is a strict prefix of another, and nothing else', async () => {
    apiFetch.mockResolvedValue([
      person('a', 'Jesse'),
      person('b', 'Jesse Sharratt'),
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
    expect(container.textContent).toContain('Retry');
    expect(container.textContent).not.toContain('No people yet.');
  });

  it('opens an existing person rather than reporting a false success', async () => {
    apiFetch.mockImplementation((_url: string, init?: { method?: string }) => {
      if (init?.method === 'POST') {
        return Promise.resolve({ person: person('a', 'Jesse Sharratt'), created: false });
      }
      return Promise.resolve([person('a', 'Jesse Sharratt')]);
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
      setter.call(input, 'Jesse Sharratt');
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
});
