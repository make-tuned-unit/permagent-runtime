/** @vitest-environment jsdom */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { PersonDetailModal } from './PersonDetailModal';
import type { Person } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const person: Person = {
  entity_uuid: 'uuid-ada',
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
  apiFetch.mockReset().mockImplementation((url: string, opts?: { method?: string; body?: string }) => {
    if (opts?.method === 'POST' && String(url).endsWith('/meetings')) {
      const body = JSON.parse(opts.body ?? '{}');
      return Promise.resolve({
        id: 'm1', entity_uuid: 'uuid-ada', title: body.title ?? 'Meeting with Ada Lovelace',
        starts_at: body.starts_at, ends_at: null, notes: body.notes ?? '',
        calendar_synced: true, project_id: body.project_id ?? null,
        follow_up_at: body.follow_up_at ?? null, follow_up_note: body.follow_up_note ?? '',
        follow_up_done: false, calendar_uid: null, created_at: 't', updated_at: 't',
      });
    }
    if (url.endsWith('/meetings')) return Promise.resolve([]);
    if (url.endsWith('/projects')) return Promise.resolve([]);
    return Promise.resolve([]);
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

describe('PersonDetailModal meetings', () => {
  it('POSTs a meeting with an RFC-3339 starts_at', async () => {
    await act(async () => root.render(
      <PersonDetailModal variant="inline" projectId={null} person={person} onClose={() => {}} />,
    ));
    const add = [...container.querySelectorAll('button')].find(b => b.getAttribute('aria-label') === 'Log a meeting');
    expect(add).toBeTruthy();
    await act(async () => add!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    const title = container.querySelector('input[aria-label="Meeting title"]') as HTMLInputElement;
    const time = container.querySelector('input[aria-label="Meeting time"]') as HTMLInputElement;
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(title, 'Coffee');
      title.dispatchEvent(new Event('input', { bubbles: true }));
      setter.call(time, '2026-08-20T15:00');
      time.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const save = [...container.querySelectorAll('button')].find(b => b.textContent === 'Log meeting')!;
    await act(async () => save.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    const call = apiFetch.mock.calls.find(([url, opts]) => String(url).endsWith('/meetings') && opts?.method === 'POST');
    expect(call).toBeTruthy();
    expect(call![0]).toBe('/api/people/uuid-ada/meetings');
    const body = JSON.parse(call![1].body);
    expect(body.title).toBe('Coffee');
    expect(body.starts_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
    expect(body.starts_at.endsWith('Z') || body.starts_at.includes('+') || body.starts_at.includes('-')).toBe(true);
    expect(body.follow_up_at).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });
});
