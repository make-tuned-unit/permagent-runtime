/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../lib/api', () => ({ apiFetch: vi.fn().mockResolvedValue([]) }));
vi.mock('./PeopleGraphCanvas', () => ({ PeopleGraph: () => null }));

import { PeopleView } from './PeopleView';
import { useCommandCenter } from '../../lib/store';
import type { Person } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  localStorage.removeItem('permagent-people-mode');
  useCommandCenter.getState().closePersonDetail();
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.getState().closePersonDetail();
});

const sample: Person = {
  entity_uuid: 'uuid-ex',
  canonical_id: 'person:example-person',
  display_name: 'Example Person',
  role: 'Director of Sales',
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
  created_at: 't',
  updated_at: 't',
};

describe('PeopleView', () => {
  it('wears the same ViewHeader as every other tab, with Graph|List as afterTitle', async () => {
    await act(async () => root.render(<PeopleView />));
    expect(container.querySelector('[data-testid="view-header"]')).toBeTruthy();
    expect(container.querySelector('[data-testid="view-title"]')?.textContent).toBe('People');
    const labels = [...container.querySelectorAll('button')].map(b => b.textContent);
    expect(labels).toEqual(expect.arrayContaining(['Graph', 'List']));
    // Graph is the default — the directory search lives in List mode.
    expect(container.textContent).not.toContain('Search people');
  });

  it('opens a side panel of always-editable fields when a person is selected', async () => {
    await act(async () => root.render(<PeopleView />));
    expect(container.querySelector('[data-testid="person-detail-panel"]')).toBeNull();

    await act(async () => {
      useCommandCenter.getState().openPersonDetail(null, sample);
    });

    const panel = container.querySelector('[data-testid="person-detail-panel"]');
    expect(panel).toBeTruthy();
    expect(panel!.textContent).toContain('Example Person');
    expect([...panel!.querySelectorAll('button')].some(b => b.textContent === 'Edit fields')).toBe(false);
    const labels = [...panel!.querySelectorAll('label')].map(l => l.textContent ?? '');
    for (const name of ['Photo', 'Role', 'Company', 'Email', 'Phone', 'LinkedIn', 'X', 'Facebook', 'Instagram', 'Site', 'Birthday', 'Last contact', 'Relationship', 'How met', 'Find online', 'Notes']) {
      expect(labels.some(t => t.startsWith(name))).toBe(true);
    }
    const role = [...panel!.querySelectorAll('label')].find(l => l.textContent?.startsWith('Role'))?.querySelector('input') as HTMLInputElement;
    expect(role.value).toBe('Director of Sales');
  });
});
