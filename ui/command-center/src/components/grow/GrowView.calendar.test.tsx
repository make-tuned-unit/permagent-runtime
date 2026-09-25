/**
 * @vitest-environment jsdom
 *
 * Grow content-calendar lens: day grouping, load-error honesty, and PATCH/DELETE
 * against /api/projects/:id/cards/:cardId (routes/cards.rs — not PUT /api/cards).
 */

import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../lib/api', () => ({
  api: { fetchGrowMediaBlob: vi.fn(() => Promise.reject(new Error('no still'))) },
  apiFetch: vi.fn(),
  extractText: vi.fn(() => ''),
  extractSystemNotice: vi.fn(() => null),
  extractThinking: vi.fn(() => ''),
  fileToBase64: vi.fn(),
  readerIngest: vi.fn(),
  getApiBaseUrl: vi.fn(() => 'http://localhost:1234'),
}));

import { GrowView } from './GrowView';
import { useCommandCenter } from '../../lib/store';
import { apiFetch } from '../../lib/api';
import { filterByQueue, groupPostsByDay, queueBucket, queueCounts, type SocialCard } from './calendarPosts';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const apiFetchMock = vi.mocked(apiFetch);

function growExtras(url: string): unknown | undefined {
  if (url === '/api/grow/higgsfield' || url === '/api/grow/postiz') {
    return { configured: false };
  }
  if (url.includes('/publisher')) {
    return { configured: false, channels: {}, pending: null };
  }
  return undefined;
}

const project = (id: string, name: string) => ({
  id,
  slug: id,
  name,
  description: '',
  status: 'active',
  rootPath: null,
  siteUrl: null,
  repoUrl: null,
  tags: [],
  metadataJson: {},
  createdAt: '',
  updatedAt: '',
  lastOpenedAt: '',
});

/** Set a controlled input's value the way React's change tracker notices. */
function setInputValue(el: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  setter?.call(el, value);
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  apiFetchMock.mockReset();
  useCommandCenter.setState({ openGrowForProject: null, workspaces: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function settleSwap() {
  await act(async () => { await new Promise((resolve) => setTimeout(resolve, 250)); });
}

async function renderGrow() {
  await act(async () => {
    root.render(<GrowView />);
  });
  await act(async () => {
    await Promise.resolve();
  });
  await settleSwap();
}

async function openCalendar() {
  const calendar = Array.from(container.querySelectorAll('[role="tab"]')).find((b) => b.textContent === 'Calendar')!;
  await act(async () => { calendar.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
}

describe('groupPostsByDay', () => {
  it('groups posts under their scheduled day, unscheduled last', () => {
    const posts: SocialCard[] = [
      { id: '1', title: 'Later', description: '', metadataJson: { scheduledFor: '2026-08-20T12:00:00Z', postStatus: 'scheduled' } },
      { id: '2', title: 'Sooner', description: '', metadataJson: { scheduledFor: '2026-08-16T09:00:00Z', postStatus: 'draft' } },
      { id: '3', title: 'No date', description: '', metadataJson: { postStatus: 'draft' } },
      { id: '4', title: 'Bad date', description: '', metadataJson: { scheduledFor: 'not-a-date' } },
    ];
    const groups = groupPostsByDay(posts);
    expect(groups[groups.length - 1].day).toBe('unscheduled');
    expect(groups[groups.length - 1].posts.map((p) => p.id).sort()).toEqual(['3', '4']);
    const scheduledTitles = groups
      .filter((g) => g.day !== 'unscheduled')
      .flatMap((g) => g.posts.map((p) => p.title));
    expect(scheduledTitles).toEqual(['Sooner', 'Later']);
  });
});

/**
 * UX_BENCHMARK_RESEARCH §9 (dagK 2026-09-25: Buffer's queue). `attention`
 * overrides the schedule status because a failed still needs a look
 * regardless of whether the post is a draft or already scheduled.
 */
describe('queueBucket / queueCounts / filterByQueue', () => {
  const draft: SocialCard = { id: '1', title: 'D', description: '', metadataJson: { postStatus: 'draft' } };
  const scheduled: SocialCard = { id: '2', title: 'S', description: '', metadataJson: { postStatus: 'scheduled' } };
  const posted: SocialCard = { id: '3', title: 'P', description: '', metadataJson: { postStatus: 'posted' } };
  const failedStill: SocialCard = {
    id: '4', title: 'F', description: '',
    metadataJson: { postStatus: 'scheduled', mediaStatus: 'failed' },
  };

  it('buckets a card by its schedule status', () => {
    expect(queueBucket(draft)).toBe('draft');
    expect(queueBucket(scheduled)).toBe('scheduled');
    expect(queueBucket(posted)).toBe('posted');
  });

  it('puts a failed-still card in attention regardless of its schedule status', () => {
    expect(queueBucket(failedStill)).toBe('attention');
  });

  it('counts every bucket', () => {
    const counts = queueCounts([draft, scheduled, posted, failedStill]);
    expect(counts).toEqual({ draft: 1, scheduled: 1, posted: 1, attention: 1 });
  });

  it('filters to one bucket, or passes everything through for "all"', () => {
    const all = [draft, scheduled, posted, failedStill];
    expect(filterByQueue(all, 'all')).toEqual(all);
    expect(filterByQueue(all, 'draft').map((p) => p.id)).toEqual(['1']);
    expect(filterByQueue(all, 'attention').map((p) => p.id)).toEqual(['4']);
  });
});

describe('Grow calendar lens', () => {
  it('renders day headings for scheduled posts', async () => {
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url.includes('/cards?card_type=social_post')) {
        return Promise.resolve([
          {
            id: 'a',
            title: 'Monday post',
            description: 'body',
            metadataJson: { scheduledFor: '2026-08-17T15:00:00Z', postStatus: 'scheduled' },
          },
        ]);
      }
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();
    expect(container.textContent).toContain('Monday post');
    expect(container.textContent).toMatch(/Aug|August|2026/);
    expect(container.textContent).toContain('scheduled');
    expect(container.textContent).not.toContain('No posts yet');
  });

  it('load failure shows error state, not empty state', async () => {
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url.includes('/cards?card_type=social_post')) {
        return Promise.reject(new Error('network down'));
      }
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();
    expect(container.textContent).toContain("Couldn't load the content calendar.");
    expect(container.textContent).not.toContain('No posts yet');
  });

  it('delete and reschedule hit PATCH/DELETE on the project card path', async () => {
    const card = {
      id: 'post-1',
      title: 'Draft',
      description: 'hi',
      metadataJson: { postStatus: 'draft', scheduledFor: '2026-08-18T12:00:00.000Z' },
    };
    apiFetchMock.mockImplementation(((url: string, init?: RequestInit) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url.includes('/cards?card_type=social_post')) return Promise.resolve([card]);
      if (url === '/api/projects/p1/cards/post-1') {
        return Promise.resolve(init?.method === 'DELETE' ? undefined : card);
      }
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();

    const schedule = container.querySelector<HTMLInputElement>('input[aria-label="Reschedule post"]')!;
    // React tracks the last value it wrote to the node, so assigning `.value`
    // directly makes it treat the event as a no-op. Go through the native
    // setter its tracker patches, then blur: the schedule field commits on
    // blur, not on every segment the user types.
    await act(async () => {
      setInputValue(schedule, '2026-08-19T10:30');
      schedule.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => {
      schedule.dispatchEvent(new FocusEvent('focusout', { bubbles: true }));
    });
    await act(async () => { await Promise.resolve(); });

    const patchCall = apiFetchMock.mock.calls.find(
      (c) => String(c[0]) === '/api/projects/p1/cards/post-1' && (c[1] as RequestInit)?.method === 'PATCH',
    );
    expect(patchCall).toBeTruthy();
    const patchBody = JSON.parse(String((patchCall![1] as RequestInit).body));
    // Blur reschedules only. Approve is the only draft → scheduled path.
    expect(patchBody.metadataJson.postStatus).toBe('draft');
    expect(patchBody.metadataJson.scheduledFor).toMatch(/2026-08-19/);

    const delBtn = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Delete')!;
    await act(async () => { delBtn.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await act(async () => { await Promise.resolve(); });

    expect(apiFetchMock.mock.calls.some(
      (c) => String(c[0]) === '/api/projects/p1/cards/post-1' && (c[1] as RequestInit)?.method === 'DELETE',
    )).toBe(true);
  });

  it('Approve is offered only when a draft still is ready, and posts /approve', async () => {
    const card = {
      id: 'post-2',
      title: 'Ready draft',
      description: 'hi',
      metadataJson: {
        postStatus: 'draft',
        scheduledFor: '2026-08-18T12:00:00.000Z',
        mediaStatus: 'ready',
        media: [{ kind: 'still', file: 'still.png' }],
      },
    };
    apiFetchMock.mockImplementation(((url: string, init?: RequestInit) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url === '/api/grow/higgsfield') return Promise.resolve({ configured: false });
      if (url.includes('/cards?card_type=social_post')) return Promise.resolve([card]);
      if (url === '/api/projects/p1/cards/post-2/approve' && init?.method === 'POST') {
        return Promise.resolve({ ...card, metadataJson: { ...card.metadataJson, postStatus: 'scheduled' } });
      }
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();
    const approve = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Approve');
    expect(approve).toBeTruthy();
    await act(async () => { approve!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await act(async () => { await Promise.resolve(); });
    expect(apiFetchMock.mock.calls.some(
      (c) => String(c[0]) === '/api/projects/p1/cards/post-2/approve' && (c[1] as RequestInit)?.method === 'POST',
    )).toBe(true);
  });

  // Approve on a network the project hasn't connected still marks the post
  // scheduled, but nothing publishes. The card used to say READY + Approve.
  it('says Approve will not publish when the post\'s network is not connected', async () => {
    const igCard = (id: string) => ({
      id,
      title: 'IG draft',
      description: 'hi',
      metadataJson: {
        postStatus: 'draft', scheduledFor: '2026-08-18T12:00:00.000Z', channel: 'ig',
        mediaStatus: 'ready', media: [{ kind: 'still', file: 'still.png' }],
      },
    });
    const run = async (channels: Record<string, unknown>) => {
      apiFetchMock.mockImplementation(((url: string) => {
        if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
        if (url.includes('/cards?card_type=social_post')) return Promise.resolve([igCard('post-ig')]);
        if (url === '/api/projects/p1/publisher') {
          return Promise.resolve({ configured: true, channels, pending: null });
        }
        return Promise.resolve(growExtras(url) ?? []);
      }) as typeof apiFetch);
      await renderGrow();
      await openCalendar();
      await act(async () => { await Promise.resolve(); });
    };

    await run({});
    const labels = Array.from(container.querySelectorAll('button')).map((b) => b.textContent);
    expect(labels).toContain("Approve (won't publish)");
    expect(container.querySelector('[role="note"]')?.textContent).toContain("Instagram isn't connected");

    act(() => root.unmount());
    root = createRoot(container);
    await run({ ig: { integrationId: 'int-1', name: 'evntally' } });
    const connected = Array.from(container.querySelectorAll('button')).map((b) => b.textContent);
    expect(connected).toContain('Approve');
    expect(container.querySelector('[role="note"]')).toBeNull();
  });

  it('does not offer Approve while the still is still generating', async () => {
    const card = {
      id: 'post-3',
      title: 'Queued',
      description: 'hi',
      metadataJson: { postStatus: 'draft', mediaStatus: 'generating' },
    };
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url === '/api/grow/higgsfield') return Promise.resolve({ configured: false });
      if (url.includes('/cards?card_type=social_post')) return Promise.resolve([card]);
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();
    expect(Array.from(container.querySelectorAll('button')).some((b) => b.textContent === 'Approve')).toBe(false);
    expect(Array.from(container.querySelectorAll('button')).some((b) => b.textContent === 'Regenerate still')).toBe(false);
  });

  it('regenerates the still with taste notes and does not rewrite copy', async () => {
    const card = {
      id: 'post-4',
      title: 'Keep this hook',
      description: 'Keep this body.',
      metadataJson: {
        postStatus: 'draft',
        scheduledFor: '2026-08-18T12:00:00.000Z',
        mediaStatus: 'ready',
        media: [{ kind: 'still', file: 'still.png' }],
      },
    };
    apiFetchMock.mockImplementation(((url: string, init?: RequestInit) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url === '/api/grow/higgsfield') return Promise.resolve({ configured: false });
      if (url.includes('/cards?card_type=social_post')) return Promise.resolve([card]);
      if (url === '/api/projects/p1/cards/post-4/media/retry' && init?.method === 'POST') {
        return Promise.resolve(card);
      }
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();
    expect(container.textContent).toContain('Keep this hook');
    expect(container.textContent).toContain('Keep this body.');
    const notes = container.querySelector<HTMLTextAreaElement>('textarea[aria-label="Still taste notes"]')!;
    await act(async () => {
      const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
      setter?.call(notes, 'darker, less type');
      notes.dispatchEvent(new Event('input', { bubbles: true }));
    });
    const regen = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Regenerate still')!;
    await act(async () => { regen.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
    await act(async () => { await Promise.resolve(); });

    const retryCall = apiFetchMock.mock.calls.find(
      (c) => String(c[0]) === '/api/projects/p1/cards/post-4/media/retry' && (c[1] as RequestInit)?.method === 'POST',
    );
    expect(retryCall).toBeTruthy();
    expect(JSON.parse(String((retryCall![1] as RequestInit).body))).toEqual({ feedback: 'darker, less type' });
    expect(apiFetchMock.mock.calls.some(
      (c) => String(c[0]) === '/api/projects/p1/cards/post-4' && (c[1] as RequestInit)?.method === 'PATCH',
    )).toBe(false);
    expect(container.textContent).toContain('Keep this hook');
    expect(container.textContent).toContain('Keep this body.');
  });

  it('filters the visible posts by the status tab, and counts every bucket', async () => {
    const cards = [
      { id: 'd1', title: 'A draft', description: '', metadataJson: { postStatus: 'draft' } },
      { id: 's1', title: 'A scheduled post', description: '', metadataJson: { postStatus: 'scheduled', scheduledFor: '2026-08-20T12:00:00Z' } },
      { id: 'p1card', title: 'A posted post', description: '', metadataJson: { postStatus: 'posted' } },
    ];
    apiFetchMock.mockImplementation(((url: string) => {
      if (url === '/api/projects') return Promise.resolve([project('p1', 'First')]);
      if (url.includes('/cards?card_type=social_post')) return Promise.resolve(cards);
      return Promise.resolve(growExtras(url) ?? []);
    }) as typeof apiFetch);

    await renderGrow();
    await openCalendar();

    const tabs = () => Array.from(container.querySelectorAll('[data-testid="queue-filter"] [role="tab"]'));
    expect(tabs().map((t) => t.textContent)).toEqual(['All (3)', 'Draft (1)', 'Scheduled (1)', 'Posted (1)']);
    // No card here has a failed still, so "Needs attention" earns no tab.
    expect(tabs().some((t) => t.textContent?.includes('Needs attention'))).toBe(false);

    expect(container.textContent).toContain('A draft');
    expect(container.textContent).toContain('A scheduled post');
    expect(container.textContent).toContain('A posted post');

    const draftTab = tabs().find((t) => t.textContent === 'Draft (1)')!;
    await act(async () => { draftTab.dispatchEvent(new MouseEvent('click', { bubbles: true })); });

    expect(container.textContent).toContain('A draft');
    expect(container.textContent).not.toContain('A scheduled post');
    expect(container.textContent).not.toContain('A posted post');
    expect(draftTab.getAttribute('aria-selected')).toBe('true');
  });
});
