/** @vitest-environment jsdom */

/**
 * GM1 (dagK): the digest's cost line used to be the only place a verifier
 * run's `worker_session_id` showed up — inside the raw `<pre>` disclosure, as
 * plain text (S2: no links inside that block). This proves the visible
 * summary layer gets an actual "Open transcript" click-through instead, and
 * that the digest's per-file diff (a benchmark follow-up) renders in that same
 * visible layer without waiting on "Show details".
 */

import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import type { EvidenceDigestData } from './types';

const state = vi.hoisted(() => ({
  switchToSession: vi.fn().mockResolvedValue(undefined),
  openChatDock: vi.fn(),
}));
vi.mock('../../../lib/store', () => ({
  useCommandCenter: (select: (s: Record<string, unknown>) => unknown) => select({
    switchToSession: state.switchToSession,
    openChatDock: state.openChatDock,
  }),
}));

const client = vi.hoisted(() => ({
  evidence: vi.fn(),
  dispatchEvidence: vi.fn(),
}));
vi.mock('./client', () => ({ decisionsClient: client }));

import { EvidenceDigest } from './EvidenceDigest';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;
let root: Root;
let container: HTMLDivElement;
beforeEach(() => {
  state.switchToSession.mockClear();
  state.openChatDock.mockClear();
  client.evidence.mockReset();
  client.dispatchEvidence.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const DIGEST: EvidenceDigestData = {
  version: 1,
  goal_id: 'g1',
  goal_title: 'Ship it',
  checks_summary: { total_count: 1, passed_count: 1, one_line: 'All checks passed' },
  verifier_summary: 'Verifier passed',
  costs: {
    cost_usd: 1.23,
    accumulated_total_tokens: null,
    note: null,
    attempt_count: 1,
    worker_session_id: 'sess-xyz',
  },
  checks: [],
  diff: {
    files_changed: 2, insertions: 10, deletions: 3,
    per_file: [
      { path: 'src/foo.ts', insertions: 8, deletions: 1 },
      { path: 'src/bar.ts', insertions: 2, deletions: 2 },
    ],
  },
  out_of_path_files: [],
  verifier: { status: 'pass', rationale: '', model: 'test-model' },
  started_at: '2026-01-01T00:00:00Z',
  finished_at: '2026-01-01T00:05:00Z',
} as unknown as EvidenceDigestData;

async function settle() {
  await act(async () => { await Promise.resolve(); await Promise.resolve(); await Promise.resolve(); });
}

it('GM1: a worker session on the digest is an Open-transcript click-through, in the visible summary layer', async () => {
  client.evidence.mockResolvedValue(DIGEST);
  client.dispatchEvidence.mockResolvedValue(null);
  await act(async () => { root.render(<EvidenceDigest projectId="p1" goalId="g1" />); });
  await settle();

  const btn = Array.from(container.querySelectorAll('button')).find(b => b.textContent === 'Open transcript');
  expect(btn, 'must be reachable without opening "Show details"').toBeTruthy();
  await act(async () => { btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })); });
  expect(state.switchToSession).toHaveBeenCalledWith('sess-xyz');
  expect(state.openChatDock).toHaveBeenCalledTimes(1);
});

it('does not render an Open-transcript button when no worker session is on the digest', async () => {
  client.evidence.mockResolvedValue({ ...DIGEST, costs: { ...DIGEST.costs, worker_session_id: null } });
  client.dispatchEvidence.mockResolvedValue(null);
  await act(async () => { root.render(<EvidenceDigest projectId="p1" goalId="g1" />); });
  await settle();

  expect(Array.from(container.querySelectorAll('button')).some(b => b.textContent === 'Open transcript')).toBe(false);
});

it('benchmark follow-up: the per-file diff renders in the visible summary layer, not only inside "Show details"', async () => {
  client.evidence.mockResolvedValue(DIGEST);
  client.dispatchEvidence.mockResolvedValue(null);
  await act(async () => { root.render(<EvidenceDigest projectId="p1" goalId="g1" />); });
  await settle();

  expect(container.textContent).toContain('src/foo.ts');
  expect(container.textContent).toContain('src/bar.ts');
  // Confirm it is OUTSIDE the collapsed disclosure: the toggle still reads
  // "Show details" (not yet expanded) while the files are already visible.
  expect(container.textContent).toContain('Show details');
});

// The compact row used to append the digest's own attempt count and duration
// after the decision's lead facts: "3 attempts · 3 attempts · …" and
// "4 min used · 1 attempt · 5 min · …". Each fact is stated once, and the
// decision's own figure wins.
async function compactRow(leadFacts: Parameters<typeof EvidenceDigest>[0]['leadFacts']) {
  client.evidence.mockResolvedValue({ ...DIGEST, costs: { ...DIGEST.costs, attempt_count: 3 } });
  client.dispatchEvidence.mockResolvedValue(null);
  await act(async () => { root.render(<EvidenceDigest compact projectId="p1" goalId="g1" leadFacts={leadFacts} />); });
  await settle();
  return container.querySelector('[data-testid="evidence-facts"]')!.textContent!;
}

it('compact: an attempt-cap lead replaces the digest\'s attempt count instead of repeating it', async () => {
  const row = await compactRow({ attempts: 3 });
  expect(row.match(/attempt/g)).toHaveLength(1);
  expect(row.startsWith('3 attempts · 5 min')).toBe(true);
});

it('compact: a time-used lead replaces the digest\'s duration instead of repeating it', async () => {
  const row = await compactRow({ timeUsed: '4 min' });
  expect(row).toContain('4 min used');
  expect(row).not.toContain('5 min');
  expect(row.match(/attempt/g)).toHaveLength(1);
});

it('compact: with no evidence recorded, the lead facts still show on their own', async () => {
  client.evidence.mockResolvedValue(null);
  client.dispatchEvidence.mockResolvedValue(null);
  await act(async () => { root.render(<EvidenceDigest compact projectId="p1" goalId="g1" leadFacts={{ attempts: 2 }} />); });
  await settle();
  expect(container.querySelector('[data-testid="evidence-facts"]')!.textContent).toBe('2 attempts');
});
