/** @vitest-environment jsdom
 *
 * Settings → Preferences' "You" section (dagK owner-identity, 2026-09-26):
 * before this existed there was no concept of who the user is anywhere in the
 * app, and People → List showed the operator's own directory record twice —
 * their name entered once here, and once as an ordinary contact flagged as a
 * possible duplicate of itself. This pins the section's own behaviour: the
 * save flow (dirty tracking, #167's "only clear dirty on a confirmed save"),
 * and the link / unlink flow for "This is me in People".
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: {
    getDevRoots: vi.fn(() => Promise.resolve({ confirmed: [], discovered: [], home: '/Users/x' })),
    checkDevRoot: vi.fn(),
    upsertConfig: vi.fn(),
    getOwner: vi.fn(),
    putOwner: vi.fn(),
  },
  apiFetch: vi.fn(),
}));

vi.mock('../../lib/notifications', () => ({
  KIND_LABELS: {},
  getNotificationPrefs: vi.fn(() => ({})),
  setNotificationPref: vi.fn(),
  getOsNotificationsEnabled: vi.fn(() => false),
  setOsNotificationsEnabled: vi.fn(),
}));

import { api, apiFetch } from '../../lib/api';
import { PreferencesPanel } from './SettingsView';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getOwner = vi.mocked(api.getOwner);
const putOwner = vi.mocked(api.putOwner);
const apiFetchMock = vi.mocked(apiFetch);

const emptyOwner = { display_name: null, email: null, linked_person_id: null, linked_person_name: null };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  getOwner.mockReset().mockResolvedValue(emptyOwner);
  putOwner.mockReset();
  apiFetchMock.mockReset().mockResolvedValue([]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => { root.render(<PreferencesPanel />); });
  // Flush the owner load's microtask chain.
  await act(async () => { await Promise.resolve(); await Promise.resolve(); });
}

function textInputs(): HTMLInputElement[] {
  return Array.from(container.querySelectorAll('input')).filter(i => i.type !== 'checkbox' && i.type !== 'radio');
}

function buttonMatching(pattern: string): HTMLButtonElement {
  const btns = Array.from(container.querySelectorAll('button'));
  const btn = btns.find(b => b.textContent?.includes(pattern));
  if (!btn) throw new Error(`no button containing "${pattern}" — have: ${btns.map(b => b.textContent).join(' | ')}`);
  return btn;
}

async function click(btn: HTMLButtonElement) {
  await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

function setValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value')!.set!;
  setter.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('Preferences → You', () => {
  it('renders the section with empty name/email and a disabled Save until edited', async () => {
    await render();
    expect(container.textContent).toContain('You');
    const save = buttonMatching('Save');
    expect(save.disabled).toBe(true);
    expect(save.getAttribute('data-disabled-reason')).toBe('no changes to save');
  });

  it('loads a previously-saved name/email into the fields', async () => {
    getOwner.mockResolvedValue({ ...emptyOwner, display_name: 'Ada Lovelace', email: 'ada@example.com' });
    await render();
    const [nameInput] = textInputs();
    expect(nameInput.value).toBe('Ada Lovelace');
  });

  it('enables Save once the name is edited, and saves only after a confirmed PUT', async () => {
    putOwner.mockResolvedValue({ ...emptyOwner, display_name: 'Ada Lovelace' });
    await render();
    const [nameInput] = textInputs();
    setValue(nameInput, 'Ada Lovelace');

    const save = buttonMatching('Save');
    expect(save.disabled).toBe(false);
    await click(save);

    expect(putOwner).toHaveBeenCalledWith({ display_name: 'Ada Lovelace', email: '' });
    // #167: dirty clears only because the mocked PUT resolved.
    expect(buttonMatching('Save').disabled).toBe(true);
  });

  it('does not clear the dirty flag when the save fails', async () => {
    putOwner.mockRejectedValue(new Error('network down'));
    await render();
    const [nameInput] = textInputs();
    setValue(nameInput, 'Ada Lovelace');
    await click(buttonMatching('Save'));

    expect(buttonMatching('Save').disabled).toBe(false);
    expect(container.textContent).toContain('network down');
  });

  it('offers a person search once "Link a person" is clicked, and links on pick', async () => {
    const jane = {
      entity_uuid: 'uuid-jane', canonical_id: 'person:jane', display_name: 'Jane Doe',
      role: null, company: null, email: null, phone: null, notes: null, last_contact_at: null,
      birthday: null, relationship_strength: null, how_met: null, linkedin: null, x_handle: null,
      facebook: null, instagram: null, personal_site: null, photo_url: null, find_online_hints: null,
      created_at: '', updated_at: '',
    };
    apiFetchMock.mockResolvedValue([jane]);
    putOwner.mockResolvedValue({ ...emptyOwner, linked_person_id: 'uuid-jane', linked_person_name: 'Jane Doe' });
    await render();

    await click(buttonMatching('Link a person'));
    expect(apiFetchMock).toHaveBeenCalledWith('/api/people');
    await act(async () => { await Promise.resolve(); });
    expect(container.textContent).toContain('Jane Doe');

    await click(buttonMatching('Jane Doe'));
    expect(putOwner).toHaveBeenCalledWith({ linked_person_id: 'uuid-jane' });

    // The picker is gone; the linked row (with an Unlink action) took its place.
    expect(container.querySelector('[data-testid="owner-link-start"]')).toBeFalsy();
    expect(container.querySelector('[data-testid="owner-unlink"]')).toBeTruthy();
  });

  it('shows the linked person and unlinks on request', async () => {
    getOwner.mockResolvedValue({ ...emptyOwner, linked_person_id: 'uuid-jane', linked_person_name: 'Jane Doe' });
    putOwner.mockResolvedValue(emptyOwner);
    await render();

    expect(container.querySelector('[data-testid="owner-linked-person"]')?.textContent).toContain('Jane Doe');

    await click(buttonMatching('Unlink'));
    expect(putOwner).toHaveBeenCalledWith({ linked_person_id: null });
  });

  it('flags a linked id that no longer resolves to a person, rather than silently showing nothing', async () => {
    getOwner.mockResolvedValue({ ...emptyOwner, linked_person_id: 'uuid-gone', linked_person_name: null });
    await render();
    expect(container.textContent).toContain('no longer exists');
  });
});
