/**
 * @vitest-environment jsdom
 *
 * PersonDetailModal profile links (2026-08-19).
 *
 * The Enricher writes `linkedin` / `personal_site` / `job_title` into the graph
 * with `source = enriched`. Until this change the backend read overlay had no
 * mapping for those names, so an approved enrichment was stored and the person
 * view showed nothing new. These pin the two halves of the user-visible fix:
 *
 *  - an enriched profile link RENDERS, and clicking it deep-links the in-app
 *    browser on the Build tab (`pendingBrowserUrl`) rather than leaving the app;
 *  - `job_title` reaches the view through the backend's `role` slot, so the
 *    Role row shows the enriched title;
 *  - a non-http(s) value is NOT turned into a click target.
 *
 * On the pre-fix code the wire type carried no `linkedin`, the modal rendered no
 * link row, and these fail.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));

vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import { useCommandCenter } from '../../lib/store';
import type { ProjectPerson } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const PROFILE_URL = 'https://www.linkedin.com/in/example-person';

/** A person as the overlay now returns one: `role` carries what the Enricher
 *  proposed as `job_title`, and the profile links are populated. */
const person: ProjectPerson = {
  entity_uuid: 'uuid-1',
  canonical_id: 'person:example-person',
  display_name: 'Example Person',
  role: 'Director of Field Operations',
  company: 'Example Industries',
  email: null,
  phone: null,
  notes: null,
  last_contact_at: null,
  birthday: null,
  relationship_strength: null,
  how_met: null,
  linkedin: PROFILE_URL,
  x_handle: null,
  facebook: null,
  instagram: null,
  personal_site: null,
  photo_url: null,
  find_online_hints: null,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  project_role: null,
  associated_at: '2026-01-01T00:00:00Z',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockResolvedValue([]);
  useCommandCenter.setState({ pendingBrowserUrl: null });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render(p: ProjectPerson) {
  await act(async () => root.render(
    <PersonDetailModal projectId={null} person={p} onClose={() => {}} />,
  ));
}

function findButton(text: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll('button')]
    .find(b => b.textContent?.includes(text)) as HTMLButtonElement | undefined;
}

describe('PersonDetailModal profile links', () => {
  it('opens an enriched LinkedIn URL in the in-app browser on the Build tab', async () => {
    await render(person);

    const btn = findButton('Open profile');
    expect(btn, 'enriched linkedin must render a click target').toBeTruthy();
    // Not an anchor: an <a href> would hand the URL to the system browser.
    expect(btn!.tagName).toBe('BUTTON');
    expect(container.querySelector(`a[href="${PROFILE_URL}"]`)).toBeNull();

    await act(async () => btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    // `openInBrowser` posts the URL for the Build tab's browser to consume.
    expect(useCommandCenter.getState().pendingBrowserUrl).toBe(PROFILE_URL);
  });

  it('shows the enriched job title in the Role field', async () => {
    await render(person);
    const lbl = [...container.querySelectorAll('label')].find(l => l.textContent?.startsWith('Role'));
    expect((lbl?.querySelector('input') as HTMLInputElement | null)?.value).toBe('Director of Field Operations');
  });

  it('opens an enriched Facebook URL from the Facebook field', async () => {
    const url = 'https://www.facebook.com/example.person';
    await render({ ...person, facebook: url });
    const btn = [...container.querySelectorAll('label')]
      .find(l => l.textContent?.startsWith('Facebook'))
      ?.querySelector('button');
    expect(btn?.textContent).toContain('Open profile');
    await act(async () => btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(useCommandCenter.getState().pendingBrowserUrl).toBe(url);
  });

  it('does not make a non-http(s) value clickable', async () => {
    await render({ ...person, linkedin: 'javascript:alert(1)' });
    expect(findButton('Open profile')).toBeUndefined();
    expect(useCommandCenter.getState().pendingBrowserUrl).toBeNull();
  });
});
