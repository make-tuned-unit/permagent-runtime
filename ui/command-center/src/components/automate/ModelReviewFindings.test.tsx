/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { beforeEach, afterEach, expect, it, vi } from 'vitest';
import { ModelReviewFindings } from './ModelReviewFindings';
import { groupModelFindings, snoozeUntil, type ModelFindingRow } from './modelFindingLedger';
import { RunDetail } from './AutomateView';
import { apiFetch } from '../../lib/api';

vi.mock('../../lib/api', async importOriginal => ({
  ...await importOriginal<typeof import('../../lib/api')>(), apiFetch: vi.fn(),
}));
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

// A key with quotes and a comma: the contract's own shape (a JSON array),
// which the UI must never parse or re-encode.
const QUOTED_KEY = '["librarian","localmodel:7b","qwen3.5:4b","evaluate"]';

const row = (key: string, state: ModelFindingRow['state'], over: Partial<ModelFindingRow> = {}, consumer = 'librarian'): ModelFindingRow => ({
  key, report_id: 'rep-1', state, defer_until: null, decision_id: null, updated_at: null,
  finding: {
    consumer, tier: 'candidate_worth_evaluating', action: 'evaluate',
    discovery: {
      source: 'ollama_library', snapshot_taken_at: '2026-09-20T00:00:00Z',
      incumbent_identity: 'localmodel:7b', candidate_identity: 'qwen3.5:4b',
      quantization: 'Q4_K_M', candidate_quantization: 'Q4_K_M',
      compatibility: 'fits_memory', size: 4_700_000_000, candidate_size: 2_600_000_000,
      uncertainty: 'Not benchmarked on the Librarian workload.', stale: false,
    },
  },
  ...over,
});

const ledger: ModelFindingRow[] = [
  row(QUOTED_KEY, 'open'),
  row('["planner","a","b","update"]', 'open', {
    finding: { ...row('x', 'open').finding, consumer: 'planner', tier: 'update_available',
      discovery: { ...row('x', 'open').finding.discovery, incumbent_identity: 'model-a', candidate_identity: 'model-b', stale: true } },
  }, 'planner'),
  row('["voice","c","d","evaluate"]', 'deferred', { defer_until: '2026-10-01T00:00:00Z' }, 'voice'),
  row('["zeta","e","f","blocked"]', 'dismissed', { decision_id: 'dec_123' }, 'zeta'),
];

let container: HTMLDivElement;
let root: Root;
let posts: Array<{ keys: string[]; action: string; defer_until?: string }>;
let postResponse: () => unknown;

function mockLedger(rows: ModelFindingRow[]) {
  vi.mocked(apiFetch).mockImplementation(async (path, options) => {
    if (path === '/api/model-review/findings/actions') {
      expect(options?.method).toBe('POST');
      posts.push(JSON.parse(String(options?.body)));
      return postResponse();
    }
    if (path === '/api/model-review/findings') return rows;
    return {};
  });
}

const buttons = () => [...container.querySelectorAll('button')];
const button = (text: string, scope: ParentNode = container) =>
  [...scope.querySelectorAll('button')].find(b => b.textContent === text);
const rowsEls = () => [...container.querySelectorAll('[data-testid="model-finding-row"]')] as HTMLElement[];

beforeEach(() => {
  vi.clearAllMocks();
  posts = [];
  postResponse = () => ({ updated: [], unknown: [] });
  container = document.createElement('div'); document.body.appendChild(container); root = createRoot(container);
});
afterEach(() => { act(() => root.unmount()); container.remove(); });

async function renderFindings(rows: ModelFindingRow[] = ledger) {
  mockLedger(rows);
  await act(async () => root.render(<ModelReviewFindings />));
}

it('groups as Open / Deferred / Dismissed and keeps the server order within each group', async () => {
  const shuffledOpen = [row('["b"]', 'open', {}, 'b-role'), row('["a"]', 'open', {}, 'a-role')];
  expect(groupModelFindings([...shuffledOpen, ledger[3], ledger[2]]).map(g => [g.label, g.rows.map(r => r.key)]))
    .toEqual([['Open', ['["b"]', '["a"]']], ['Deferred', [ledger[2].key]], ['Dismissed', [ledger[3].key]]]);

  await renderFindings();
  const sections = [...container.querySelectorAll('section')].map(s => s.getAttribute('aria-label'));
  expect(sections).toEqual(['Open', 'Deferred', 'Dismissed']);
  expect(rowsEls().map(el => el.getAttribute('data-state'))).toEqual(['open', 'open', 'deferred', 'dismissed']);
  const text = container.textContent!;
  expect(text.indexOf('librarian')).toBeLessThan(text.indexOf('planner'));
  // Row content: consumer, identities, tier label, compatibility, uncertainty, stale.
  expect(text).toContain('localmodel:7b → qwen3.5:4b');
  expect(text).toContain('Worth evaluating');
  expect(text).toContain('Update available');
  expect(text).toContain('Compatibility: fits_memory');
  expect(text).toContain('Not benchmarked on the Librarian workload.');
  expect(rowsEls()[1].textContent).toContain('stale');
  expect(rowsEls()[0].textContent).not.toContain('stale');
  expect(container.querySelector('time[datetime="2026-10-01T00:00:00Z"]')).not.toBeNull();
  // Checkboxes only on open rows; Reopen only on the others.
  expect(container.querySelectorAll('input[type="checkbox"]')).toHaveLength(2);
  expect(button('Reopen', rowsEls()[2])).toBeTruthy();
  expect(button('Reopen', rowsEls()[3])).toBeTruthy();
  expect(button('Dismiss', rowsEls()[2])).toBeUndefined();
});

it('shows the empty state', async () => {
  await renderFindings([]);
  expect(container.textContent).toContain('No model findings.');
});

it('Dismiss posts one key verbatim, then refetches', async () => {
  await renderFindings();
  const gets = () => vi.mocked(apiFetch).mock.calls.filter(c => c[0] === '/api/model-review/findings').length;
  expect(gets()).toBe(1);
  await act(async () => button('Dismiss', rowsEls()[0])!.click());
  expect(posts).toEqual([{ keys: [QUOTED_KEY], action: 'dismiss' }]);
  expect(gets()).toBe(2);
});

it('Snooze posts action defer with a future RFC3339 defer_until inside 180 days', async () => {
  await renderFindings();
  await act(async () => button('Snooze', rowsEls()[0])!.click());
  const before = Date.now();
  await act(async () => button('30 days', rowsEls()[0])!.click());
  expect(posts).toHaveLength(1);
  expect(posts[0].keys).toEqual([QUOTED_KEY]);
  expect(posts[0].action).toBe('defer');
  const until = posts[0].defer_until!;
  expect(until).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
  const t = Date.parse(until);
  expect(t).toBeGreaterThan(before);
  expect(t).toBeLessThan(before + 180 * 86_400_000);
  expect(snoozeUntil(90, 0)).toBe('1970-04-01T00:00:00Z');
});

it('bulk Dismiss and Snooze post every selected key in one request', async () => {
  await renderFindings();
  expect(button('Dismiss selected')).toBeUndefined();
  for (const box of container.querySelectorAll('input[type="checkbox"]')) {
    await act(async () => (box as HTMLInputElement).click());
  }
  await act(async () => button('Dismiss selected')!.click());
  expect(posts).toEqual([{ keys: [QUOTED_KEY, '["planner","a","b","update"]'], action: 'dismiss' }]);

  // The refetch returned the same ledger (both still open), so the selection survives.
  await act(async () => button('Snooze selected')!.click());
  await act(async () => button('1 week', container.querySelector('[data-testid="model-findings-bulk"]')!)!.click());
  expect(posts).toHaveLength(2);
  expect(posts[1].keys).toEqual([QUOTED_KEY, '["planner","a","b","update"]']);
  expect(posts[1].action).toBe('defer');
  expect(Date.parse(posts[1].defer_until!)).toBeGreaterThan(Date.now());
});

it('Reopen posts the reopen action', async () => {
  await renderFindings();
  await act(async () => button('Reopen', rowsEls()[3])!.click());
  expect(posts).toEqual([{ keys: ['["zeta","e","f","blocked"]'], action: 'reopen' }]);
});

it('shows how many keys were no longer in the ledger', async () => {
  postResponse = () => ({ updated: [], unknown: [QUOTED_KEY, 'other'] });
  await renderFindings();
  await act(async () => button('Dismiss', rowsEls()[0])!.click());
  expect(container.textContent).toContain('2 findings were no longer in the ledger.');
});

it('shows a 400 message inline', async () => {
  postResponse = () => { throw Object.assign(new Error('defer_until must be within 180 days'), { status: 400 }); };
  await renderFindings();
  await act(async () => button('Dismiss', rowsEls()[0])!.click());
  expect(container.querySelector('[role="alert"]')?.textContent).toBe('defer_until must be within 180 days');
});

it('shows a load error with retry', async () => {
  vi.mocked(apiFetch).mockRejectedValue(new Error('storage failure'));
  await act(async () => root.render(<ModelReviewFindings />));
  expect(container.textContent).toContain("Couldn't load model findings: storage failure");
  expect(button('Retry')).toBeTruthy();
});

it('offers Open in Decision Review only when decision_id is set', async () => {
  await renderFindings();
  const els = rowsEls();
  for (const el of els.slice(0, 3)) expect(button('Open in Decision Review', el)).toBeUndefined();
  expect(button('Open in Decision Review', els[3])).toBeTruthy();
  expect(buttons().filter(b => b.textContent === 'Open in Decision Review')).toHaveLength(1);
});

it('never says Trash, Keep or Plan evaluation', async () => {
  await renderFindings();
  await act(async () => button('Snooze', rowsEls()[0])!.click());
  for (const word of ['Trash', 'Keep', 'Plan evaluation']) expect(container.textContent).not.toContain(word);
});

const run = {
  id: 'run-1', jobId: 'job-1', name: 'Run', createdAt: '2026-09-24T09:00:00Z',
  workingDir: '/', scheduleId: 'job-1', messageCount: 2, totalTokens: 10, inputTokens: 5, outputTokens: 5,
};

it('RunDetail mounts ModelReviewFindings for the model_review starter, not FindingsPanel', async () => {
  const storage = [{
    id: 'f1', path: '/fixture/cache', type: 'dev_cache', category: 'safe_to_remove', size_bytes: 1024,
    age_days: null, recommendation: 'safe', consequence: null, action_taken: null, actioned_at: null, size_recovered_bytes: null,
  }];
  vi.mocked(apiFetch).mockImplementation(async path => {
    if (path === '/api/model-review/findings') return ledger;
    if (String(path).endsWith('/findings')) return { ledger_state: 'valid', findings: storage };
    if (String(path).startsWith('/automation/run/')) return { findings: storage, availability: [] };
    return {};
  });
  await act(async () => root.render(<RunDetail run={run} displayName="Model Review" starterId="model_review" />));
  expect(container.querySelector('[data-testid="model-review-findings"]')).not.toBeNull();
  expect(container.textContent).not.toContain('Trash');
  expect(container.textContent).not.toContain('cleanup findings could not be saved');
  expect(vi.mocked(apiFetch).mock.calls.some(c => String(c[0]).startsWith('/automation/run/'))).toBe(false);

  await act(async () => root.render(<RunDetail key="storage" run={run} displayName="Storage Insights" starterId="storage_insights" />));
  expect(container.querySelector('[data-testid="model-review-findings"]')).toBeNull();
  expect(container.textContent).toContain('Review cleanup options'); // FindingsPanel
  expect(vi.mocked(apiFetch).mock.calls.some(c => c[0] === '/automation/run/run-1/findings')).toBe(true);
});

it('trusts the effective state: an expired deferral is open and shows no snooze date', async () => {
  // The server reports a lapsed deferral as "open" but leaves its stored
  // defer_until in place; the UI must key off `state` alone.
  await renderFindings([row(QUOTED_KEY, 'open', { defer_until: '2026-01-01T00:00:00Z' })]);
  const [el] = rowsEls();
  expect(el.dataset.state).toBe('open');
  expect(el.textContent).not.toContain('Snoozed until');
  expect(button('Dismiss', el)).toBeTruthy();
  expect(button('Reopen', el)).toBeUndefined();
});
