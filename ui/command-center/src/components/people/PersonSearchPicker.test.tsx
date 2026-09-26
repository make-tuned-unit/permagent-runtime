/**
 * @vitest-environment jsdom
 *
 * PersonSearchPicker — debounce-free search over `GET /api/people?q=`,
 * shared by the project "+ Associate" flow and Settings → Preferences'
 * "This is me in People" (dagK owner-identity, 2026-09-26). Extracted from
 * `PeoplePanel.tsx`'s local `AssociatePicker`; this pins the behaviour that
 * extraction must not change: query wiring, exclude filtering, loading/error/
 * empty states, and that picking a row calls `onPick` with the full person.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonSearchPicker } from './PersonSearchPicker';
import type { Person } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function blankPerson(entity_uuid: string, display_name: string, extra: Partial<Person> = {}): Person {
  return {
    entity_uuid, canonical_id: `person:${entity_uuid}`, display_name,
    role: null, company: null, email: null, phone: null, notes: null,
    last_contact_at: null, birthday: null, relationship_strength: null, how_met: null,
    linkedin: null, x_handle: null, facebook: null, instagram: null, personal_site: null,
    photo_url: null, find_online_hints: null,
    created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z',
    ...extra,
  };
}

const jane = blankPerson('uuid-jane', 'Jane Doe', { role: 'Engineer', company: 'Acme' });
const john = blankPerson('uuid-john', 'John Roe');

let container: HTMLDivElement;
let root: Root;
let onPick: ReturnType<typeof vi.fn>;

beforeEach(() => {
  onPick = vi.fn();
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url === '/api/people') return Promise.resolve([jane, john]);
    if (url === '/api/people?q=jane') return Promise.resolve([jane]);
    return Promise.resolve([]);
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(excludeIds?: Set<string>) {
  await act(async () => root.render(
    <PersonSearchPicker excludeIds={excludeIds} onPick={onPick} />,
  ));
}

describe('PersonSearchPicker', () => {
  it('loads the unfiltered directory and lists role/company as the subtitle', async () => {
    await render();
    expect(apiFetch).toHaveBeenCalledWith('/api/people');
    expect(container.textContent).toContain('Jane Doe');
    expect(container.textContent).toContain('Engineer · Acme');
    expect(container.textContent).toContain('John Roe');
  });

  it('excludes ids already accounted for elsewhere', async () => {
    await render(new Set(['uuid-john']));
    expect(container.textContent).toContain('Jane Doe');
    expect(container.textContent).not.toContain('John Roe');
  });

  it('re-queries with the typed text', async () => {
    await render();
    const input = container.querySelector('input') as HTMLInputElement;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
      setter.call(input, 'jane');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect(apiFetch).toHaveBeenCalledWith('/api/people?q=jane');
    expect(container.textContent).toContain('Jane Doe');
    expect(container.textContent).not.toContain('John Roe');
  });

  it('shows a directory-empty message distinct from a filtered-to-nothing one', async () => {
    apiFetch.mockReset().mockResolvedValue([]);
    await render();
    expect(container.textContent).toContain('No people in the directory.');
  });

  it('shows a search error rather than an empty state on a failed fetch', async () => {
    apiFetch.mockReset().mockRejectedValue(new Error('network down'));
    await render();
    expect(container.textContent).toContain("Couldn't search the directory.");
  });

  it('calls onPick with the full person record when a row is clicked', async () => {
    await render();
    const btn = [...container.querySelectorAll('button')].find(b => b.textContent?.includes('Jane Doe'));
    expect(btn).toBeTruthy();
    await act(async () => btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onPick).toHaveBeenCalledWith(jane);
  });
});
