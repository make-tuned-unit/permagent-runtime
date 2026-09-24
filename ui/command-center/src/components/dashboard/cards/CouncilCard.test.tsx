/**
 * @vitest-environment jsdom
 *
 * A daemon that can't be reached is not a Council with nothing to say. The
 * card has to name what failed and offer a way back, not print a raw
 * exception and stop.
 */

import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { createRoot, type Root } from 'react-dom/client';
import { act } from 'react-dom/test-utils';

vi.mock('../../../lib/api', () => ({
  api: { getCouncilLatest: vi.fn() },
}));
vi.mock('../decisions/useDecisions', () => ({ useDecisions: () => ({}) }));
vi.mock('../decisions/DecisionInbox', () => ({ DecisionInbox: () => null }));

import { CouncilCard } from './CouncilCard';
import { MIN_PENDING_MS } from '../../common/Button';
import { api } from '../../../lib/api';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

const getLatest = vi.mocked(api.getCouncilLatest);

let container: HTMLDivElement;
let root: Root;

async function settle() {
  await act(async () => {
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

async function advance(ms: number) {
  await act(async () => {
    vi.advanceTimersByTime(ms);
    for (let i = 0; i < 8; i += 1) await Promise.resolve();
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  getLatest.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

it('names the failure in interface voice and offers a retry', async () => {
  getLatest.mockRejectedValue(new Error('connection refused'));
  await act(async () => { root.render(<CouncilCard />); });
  await settle();

  expect(container.textContent).toMatch(/Couldn't load/i);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Retry/i.test(b.textContent ?? ''));
  expect(retry).toBeTruthy();
});

it('retry refetches and clears the error once the daemon answers', async () => {
  getLatest.mockRejectedValueOnce(new Error('connection refused'));
  await act(async () => { root.render(<CouncilCard />); });
  await settle();

  getLatest.mockResolvedValue({ report: null, session: null, positions: [], openActions: 0 } as never);
  const retry = Array.from(container.querySelectorAll('button')).find((b) => /Retry/i.test(b.textContent ?? ''))!;
  await act(async () => { retry.click(); });
  await advance(MIN_PENDING_MS + 50);

  expect(getLatest).toHaveBeenCalledTimes(2);
  expect(container.textContent).not.toMatch(/Couldn't load/i);
  expect(container.textContent).toMatch(/No weekly report yet/i);
});

it('renders the report body as formatted markdown, not literal asterisks and hashes', async () => {
  // QA at real size: Home showed "**Verdict:**" and "#### 🟠 **Permagent
  // Runtime** — *Systemic infrastructure failure*" character for character.
  const markdown = [
    '**Context:** the daemon restarted three times this week.',
    '',
    '#### 🟠 **Permagent Runtime** — *Systemic infrastructure failure*',
    '',
    '**Risk:** high',
    '',
    '**Verdict:** fix the restart loop first.',
  ].join('\n');
  getLatest.mockResolvedValue({
    report: { headline: 'One systemic failure', markdown },
    session: { status: 'complete' },
    positions: [],
    openActions: 0,
  } as never);
  await act(async () => { root.render(<CouncilCard />); });
  await settle();

  const body = container.querySelector('[data-testid="council-report-body"]') as HTMLElement;
  expect(body.textContent).toContain('Verdict:');
  expect(body.textContent).not.toContain('**');
  expect(body.textContent).not.toContain('####');
  expect(Array.from(body.querySelectorAll('strong')).map(s => s.textContent)).toEqual(
    expect.arrayContaining(['Context:', 'Risk:', 'Verdict:', 'Permagent Runtime']),
  );
  expect(body.querySelector('em')?.textContent).toBe('Systemic infrastructure failure');
  // The heading is a heading, kept at the card's body size: no page-size text
  // class, only weight and spacing from the wrapper.
  const h4 = body.querySelector('h4') as HTMLElement;
  expect(h4.textContent).toContain('Permagent Runtime');
  expect(h4.style.fontSize).toBe('');
  expect(body.className).toContain('[&_h4]:font-semibold');
  expect(body.style.whiteSpace).not.toBe('pre-wrap');
});
