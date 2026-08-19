/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import type { ProjectPerson } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
const person = { entity_uuid: 'alice', display_name: 'Alice', canonical_id: 'person:alice',
  role: null, company: null, email: null, phone: null, notes: null, last_contact_at: null,
  birthday: null, relationship_strength: null, how_met: null,
  linkedin: null, x_handle: null, personal_site: null, created_at: '', updated_at: '',
  project_role: null, associated_at: '' } satisfies ProjectPerson;
let container: HTMLDivElement; let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url === '/api/people') return Promise.resolve([{ ...person, entity_uuid: 'bob', display_name: 'Bob' }]);
    if (url.endsWith('/relationships')) return Promise.resolve([{ from_entity_uuid: 'alice', to_entity_uuid: 'bob', predicate: 'manages', other_person: { ...person, entity_uuid: 'bob', display_name: 'Bob' } }]);
    if (url.endsWith('/activity')) return Promise.resolve([{ id: 'memory-1', kind: 'memory', title: 'Met Bob', detail: 'Alice met Bob', timestamp: '2026-07-22T10:00:00Z' }]);
    return Promise.resolve(undefined);
  });
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('Person detail relationships and activity', () => {
  it('renders graph relationships and real per-person activity', async () => {
    await act(async () => root.render(<PersonDetailModal projectId="p1" person={person} onClose={() => {}} />));
    expect(container.textContent).toContain('Related people');
    expect(container.textContent).toContain('Bob');
    expect(container.textContent).toContain('manages');
    expect(container.textContent).toContain('Recent activity');
    expect(container.textContent).toContain('Met Bob');
  });

  it('posts a selected typed relationship', async () => {
    await act(async () => root.render(<PersonDetailModal projectId="p1" person={person} onClose={() => {}} />));
    const add = container.querySelector('button[aria-label="Add related person"]') as HTMLButtonElement;
    await act(async () => add.click());
    const select = container.querySelector('select[aria-label="Related person"]') as HTMLSelectElement;
    await act(async () => { select.value = 'bob'; select.dispatchEvent(new Event('change', { bubbles: true })); });
    const buttons = [...container.querySelectorAll('button')];
    await act(async () => (buttons.find(b => b.textContent === 'Add') as HTMLButtonElement).click());
    expect(apiFetch).toHaveBeenCalledWith('/api/people/alice/relationships', expect.objectContaining({
      method: 'POST', body: JSON.stringify({ target_entity_uuid: 'bob', predicate: 'related_to' }),
    }));
  });
});
