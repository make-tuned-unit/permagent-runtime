/**
 * @vitest-environment jsdom
 *
 * PersonDetailModal manual field-edit (CRM slice 2b, #495) — component wiring:
 *  - "Edit fields" reveals inputs seeded from the person; Save PATCHes only the
 *    CHANGED fields to `/api/people/{entity_uuid}/fields` (the manual write path).
 *  - On success the view reconciles from the response and the store's peopleRev
 *    bumps so the decoupled panel refetches.
 *  - On a non-2xx the optimistic edit rolls back and an error is surfaced.
 *
 * These assert the real wire (endpoint, method, body, revision bump) — not a
 * decorative form — so the "no dead UI" bar is met.
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

const person: ProjectPerson = {
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
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  project_role: 'Advisor',
  associated_at: '2026-01-01T00:00:00Z',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetch.mockReset().mockImplementation((url: string) => {
    if (url.endsWith('/relationships') || url.endsWith('/activity') || url === '/api/people') return Promise.resolve([]);
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
    <PersonDetailModal projectId="p1" person={person} onClose={() => {}} />,
  ));
}

function buttonByText(text: string): HTMLButtonElement {
  const btn = [...container.querySelectorAll('button')].find(b => b.textContent === text);
  if (!btn) throw new Error(`no button "${text}" — have: ${[...container.querySelectorAll('button')].map(b => b.textContent).join(' | ')}`);
  return btn as HTMLButtonElement;
}

async function click(btn: HTMLButtonElement) {
  await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

async function setInput(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(
    el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype,
    'value',
  )!.set!;
  await act(async () => {
    setter.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

/** Grab an edit input by its label text (the label wraps the control). */
function inputForLabel(label: string): HTMLInputElement {
  const lbl = [...container.querySelectorAll('label')].find(l => l.textContent?.startsWith(label));
  if (!lbl) throw new Error(`no field labeled "${label}"`);
  return lbl.querySelector('input, textarea') as HTMLInputElement;
}

describe('PersonDetailModal manual field edit', () => {
  it('PATCHes only changed fields with the manual write endpoint, then bumps peopleRev', async () => {
    apiFetch.mockImplementation((url: string) => url.endsWith('/fields') ? Promise.resolve({
      ...person, email: 'jane@acme.com', birthday: '1990-04-01',
    }) : Promise.resolve([]));
    await render();

    await click(buttonByText('Edit fields'));
    // Seeded from the person: unchanged fields must not be sent.
    expect(inputForLabel('Role').value).toBe('CTO');

    await setInput(inputForLabel('Email'), 'jane@acme.com');
    await setInput(inputForLabel('Birthday'), '1990-04-01');
    await click(buttonByText('Save'));

    const [url, opts] = apiFetch.mock.calls.find(([url]) => String(url).endsWith('/fields'))!;
    expect(url).toBe('/api/people/uuid-jane/fields');
    expect(opts.method).toBe('PATCH');
    const body = JSON.parse(opts.body);
    // Only the two edited fields — role/company were untouched.
    expect(body).toEqual({ fields: { email: 'jane@acme.com', birthday: '1990-04-01' } });

    // Reconciled view now shows the persisted value; panel nudged to refetch.
    expect(container.textContent).toContain('jane@acme.com');
    expect(container.textContent).toContain('1990-04-01');
    expect(useCommandCenter.getState().peopleRev).toBe(1);
  });

  it('rolls back the optimistic edit and shows an error on a failed save', async () => {
    apiFetch.mockImplementation((url: string) => url.endsWith('/fields')
      ? Promise.reject(Object.assign(new Error('boom'), { status: 500 })) : Promise.resolve([]));
    await render();

    await click(buttonByText('Edit fields'));
    await setInput(inputForLabel('Company'), 'Globex');
    await click(buttonByText('Save'));

    expect(apiFetch.mock.calls.some(([url]) => String(url).endsWith('/fields'))).toBe(true);
    // Error surfaced, still in edit mode, peopleRev untouched (no phantom refetch).
    expect(container.textContent).toContain("Couldn't save changes");
    expect(container.textContent).toContain('500');
    expect(useCommandCenter.getState().peopleRev).toBe(0);
    // The draft input still holds the attempted value (edit mode stays open).
    expect(inputForLabel('Company').value).toBe('Globex');
  });
});
