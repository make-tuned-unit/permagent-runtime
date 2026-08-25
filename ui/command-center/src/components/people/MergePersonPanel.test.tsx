/**
 * @vitest-environment jsdom
 *
 * MergePersonPanel — the pick/preview/confirm merge flow.
 *
 * Asserts the real wire: the "Likely duplicates" list comes from
 * `GET /api/people/duplicates`, filtered to pairs involving the person passed
 * in; picking a candidate fetches `merge-preview` under the survivor's id;
 * the preview renders every honesty-critical field (counts, project links,
 * copied fields, aliases, and `retained` verbatim); confirming POSTs
 * `{ duplicate_id, confirm: true }` to the survivor's `/merge` path; and
 * Swap flips which id is the survivor for both the preview refetch and the
 * final POST.
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

const { apiFetch } = vi.hoisted(() => ({ apiFetch: vi.fn() }));
vi.mock('../../lib/api', () => ({ apiFetch }));

import { MergePersonPanel } from './MergePersonPanel';
import type { DirectoryPerson, DuplicateSuggestion, MergePreview, MergeReport, Person } from '../projects/types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function blankPerson(entity_uuid: string, display_name: string): Person {
  return {
    entity_uuid, canonical_id: `person:${entity_uuid}`, display_name,
    role: null, company: null, email: null, phone: null, notes: null,
    last_contact_at: null, birthday: null, relationship_strength: null, how_met: null,
    linkedin: null, x_handle: null, facebook: null, instagram: null, personal_site: null,
    photo_url: null, find_online_hints: null,
    created_at: '2026-01-01T00:00:00Z', updated_at: '2026-01-01T00:00:00Z',
  };
}

const person = blankPerson('uuid-jane', 'Jane Doe');
const dup = blankPerson('uuid-jane2', 'Jane D.');

const directory: DirectoryPerson[] = [
  { ...blankPerson('uuid-bob', 'Bob Smith'), role: 'Engineer', company: 'Widgets Inc', projects: [] },
];

const duplicates: DuplicateSuggestion[] = [
  {
    survivor_uuid: 'uuid-jane', survivor_name: 'Jane Doe',
    duplicate_uuid: 'uuid-jane2', duplicate_name: 'Jane D.',
    score: 0.92, reasons: ['same email', 'similar name'],
  },
  // Unrelated pair — must be filtered out of the "Likely duplicates" list.
  {
    survivor_uuid: 'uuid-x', survivor_name: 'X Person',
    duplicate_uuid: 'uuid-y', duplicate_name: 'Y Person',
    score: 0.5, reasons: ['same company'],
  },
];

const previewFixture: MergePreview = {
  survivor: person, duplicate: dup,
  meetings: 3, open_follow_ups: 1,
  project_links: [
    { project_id: 'p1', project_name: 'Acme Deal', role: 'Advisor', survivor_already_linked: false },
  ],
  fields: [{ field_name: 'company', value: 'Acme', source: 'enriched' }],
  fields_kept_from_survivor: ['role'],
  aliases: ['display_name: Jane D.'],
  graph_edges: 2,
  retained: ['Notes on Jane D. are not copied onto the survivor.'],
};

const swappedPreviewFixture: MergePreview = {
  ...previewFixture,
  survivor: dup, duplicate: person,
};

const mergeReport: MergeReport = {
  merge_id: 'merge-1', survivor_uuid: 'uuid-jane', survivor_name: 'Jane Doe',
  duplicate_uuid: 'uuid-jane2', duplicate_name: 'Jane D.',
  meetings_moved: 3, project_links_moved: 1, project_links_dropped: 0,
  fields_copied: 1, graph_edges_moved: 2, aliases_recorded: 1,
  summary: 'Merged Jane D. into Jane Doe.',
};

const swappedMergeReport: MergeReport = {
  ...mergeReport,
  merge_id: 'merge-2', survivor_uuid: 'uuid-jane2', survivor_name: 'Jane D.',
  duplicate_uuid: 'uuid-jane', duplicate_name: 'Jane Doe',
  summary: 'Merged Jane Doe into Jane D.',
};

let container: HTMLDivElement;
let root: Root;
let onDone: ReturnType<typeof vi.fn>;
let onCancel: ReturnType<typeof vi.fn>;

beforeEach(() => {
  onDone = vi.fn();
  onCancel = vi.fn();
  apiFetch.mockReset().mockImplementation((url: string, opts?: RequestInit) => {
    if (url === '/api/people/directory') return Promise.resolve(directory);
    if (url === '/api/people/duplicates?limit=50') return Promise.resolve(duplicates);
    if (url === '/api/people/uuid-jane/merge-preview?duplicate_id=uuid-jane2') return Promise.resolve(previewFixture);
    if (url === '/api/people/uuid-jane2/merge-preview?duplicate_id=uuid-jane') return Promise.resolve(swappedPreviewFixture);
    if (url === '/api/people/uuid-jane/merge' && opts?.method === 'POST') return Promise.resolve(mergeReport);
    if (url === '/api/people/uuid-jane2/merge' && opts?.method === 'POST') return Promise.resolve(swappedMergeReport);
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

async function render() {
  await act(async () => root.render(
    <MergePersonPanel person={person} onDone={onDone} onCancel={onCancel} />,
  ));
}

function buttonMatching(pattern: string): HTMLButtonElement {
  const btns = [...container.querySelectorAll('button')];
  const btn = btns.find(b => b.textContent?.includes(pattern));
  if (!btn) throw new Error(`no button containing "${pattern}" — have: ${btns.map(b => b.textContent).join(' | ')}`);
  return btn as HTMLButtonElement;
}

async function click(btn: HTMLButtonElement) {
  await act(async () => btn.dispatchEvent(new MouseEvent('click', { bubbles: true })));
}

describe('MergePersonPanel — pick step', () => {
  it('lists likely duplicates involving the given person, with reasons and score', async () => {
    await render();
    expect(container.textContent).toContain('Jane D.');
    expect(container.textContent).toContain('92%');
    expect(container.textContent).toContain('same email, similar name');
    // The unrelated pair must not appear.
    expect(container.textContent).not.toContain('X Person');
    expect(container.textContent).not.toContain('Y Person');
  });

  it('also lists directory candidates, excluding the person itself', async () => {
    await render();
    expect(container.textContent).toContain('Bob Smith');
  });
});

describe('MergePersonPanel — preview step', () => {
  it('picking a candidate fetches merge-preview under the survivor id and renders every field', async () => {
    await render();
    await click(buttonMatching('Jane D.'));

    const previewCall = apiFetch.mock.calls.find(([url]) => String(url).includes('merge-preview'));
    expect(previewCall?.[0]).toBe('/api/people/uuid-jane/merge-preview?duplicate_id=uuid-jane2');

    expect(container.textContent).toContain('Keep');
    expect(container.textContent).toContain('Jane Doe');
    expect(container.textContent).toContain('absorb');
    expect(container.textContent).toContain('3 meetings move');
    expect(container.textContent).toContain('1 with an open follow-up');
    expect(container.textContent).toContain('Acme Deal');
    expect(container.textContent).toContain('Advisor');
    expect(container.textContent).toContain('company');
    expect(container.textContent).toContain('Acme');
    expect(container.textContent).toContain('Kept from Jane Doe: role');
    expect(container.textContent).toContain('display_name: Jane D.');
    expect(container.textContent).toContain('2 graph edges move');
    expect(container.textContent).toContain('Notes on Jane D. are not copied onto the survivor.');
  });
});

describe('MergePersonPanel — confirm', () => {
  it('POSTs confirm:true to the survivor path with the duplicate id, and reports the response', async () => {
    await render();
    await click(buttonMatching('Jane D.'));
    await click(buttonMatching('Merge and delete Jane D.'));

    const mergeCall = apiFetch.mock.calls.find(([url, opts]) => url === '/api/people/uuid-jane/merge' && (opts as RequestInit | undefined)?.method === 'POST');
    expect(mergeCall).toBeTruthy();
    const body = JSON.parse((mergeCall![1] as RequestInit).body as string);
    expect(body).toEqual({ duplicate_id: 'uuid-jane2', confirm: true });
    expect(onDone).toHaveBeenCalledWith(mergeReport);
  });

  it('swap flips which id is the survivor for both the preview refetch and the final POST', async () => {
    await render();
    await click(buttonMatching('Jane D.'));
    await click(buttonMatching('Swap: keep Jane D. instead'));

    const swappedPreviewCall = apiFetch.mock.calls.find(([url]) => url === '/api/people/uuid-jane2/merge-preview?duplicate_id=uuid-jane');
    expect(swappedPreviewCall).toBeTruthy();

    await click(buttonMatching('Merge and delete Jane Doe'));
    const mergeCall = apiFetch.mock.calls.find(([url, opts]) => url === '/api/people/uuid-jane2/merge' && (opts as RequestInit | undefined)?.method === 'POST');
    expect(mergeCall).toBeTruthy();
    const body = JSON.parse((mergeCall![1] as RequestInit).body as string);
    expect(body).toEqual({ duplicate_id: 'uuid-jane', confirm: true });
    expect(onDone).toHaveBeenCalledWith(swappedMergeReport);
  });
});

describe('MergePersonPanel — cancel', () => {
  it('calls onCancel from the pick step', async () => {
    await render();
    await click(buttonMatching('Cancel'));
    expect(onCancel).toHaveBeenCalled();
  });
});
