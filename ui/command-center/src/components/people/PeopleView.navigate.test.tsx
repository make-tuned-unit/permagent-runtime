/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch: apiFetch }));
vi.mock('./PeopleGraphCanvas', () => ({ PeopleGraph: () => null }));

import { PeopleView } from './PeopleView';
import { useCommandCenter } from '../../lib/store';
import type { Person } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const ada: Person = {
  entity_uuid: 'u-ada',
  canonical_id: 'person:ada-lovelace',
  display_name: 'Ada Lovelace',
  role: null, company: null, email: null, phone: null, notes: null,
  last_contact_at: null, birthday: null, relationship_strength: null, how_met: null,
  linkedin: null, x_handle: null, facebook: null, instagram: null, personal_site: null,
  photo_url: null, find_online_hints: null, created_at: 't', updated_at: 't',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url === '/api/people' || url.startsWith('/api/people?')) return Promise.resolve([ada]);
    return Promise.resolve([]);
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useCommandCenter.setState({ pendingPersonNavigation: null, personDetail: null });
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useCommandCenter.setState({ pendingPersonNavigation: null, personDetail: null });
});

describe('PeopleView agent navigation', () => {
  it('opens the named person and clears the pending seam', async () => {
    await act(async () => root.render(<PeopleView />));
    await act(async () => {
      useCommandCenter.getState().setPendingPersonNavigation({ person: 'Ada Lovelace' });
    });
    await act(async () => { await Promise.resolve(); });
    expect(useCommandCenter.getState().pendingPersonNavigation).toBeNull();
    expect(useCommandCenter.getState().personDetail?.person.entity_uuid).toBe('u-ada');
    expect(container.querySelector('[data-testid="person-detail-panel"]')).toBeTruthy();
  });
});
