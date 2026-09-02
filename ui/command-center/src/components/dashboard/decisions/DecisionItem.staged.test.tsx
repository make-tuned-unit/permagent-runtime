/** @vitest-environment jsdom */
/**
 * The commit surface for a spoken verdict (D29).
 *
 * Voice cannot authenticate — NIST SP 800-63B-4 §3.2.3.2 — so the daemon
 * stages what was said against a decision that is still open, and THIS row is
 * where it becomes an answer. What the tests pin: the row says what was said
 * and when, it never claims the answer already happened, one tap commits
 * exactly that verdict through the ordinary answer path, and the discard is
 * right beside it.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';

vi.mock('../../../lib/store', () => ({
  useCommandCenter: (sel: (s: { discussDecision: () => void; openGoalDetail: () => void }) => unknown) =>
    sel({ discussDecision: () => {}, openGoalDetail: () => {} }),
}));
vi.mock('../../../lib/notifications', () => ({ toast: () => undefined }));
vi.mock('../../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('./client', () => ({ decisionsClient: {} }));

import { DecisionItem } from './DecisionItem';
import type { Decision } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** A Tier-2 risk gate — the tier a spoken "yes" used to be able to clear. */
function stagedDecision(overrides: Partial<Decision> = {}): Decision {
  return {
    id: 'd-staged-1',
    kind: 'risk_gate',
    goal_id: null,
    project_id: 'proj-1',
    tier: 2,
    headline: 'Allow a shell command to run',
    detail: 'cc_shell: rm -rf ./build',
    payload: { action_class: 'cc_shell', summary: 'remove the build directory' },
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    created_at: '2026-08-31T00:00:00Z',
    resolved_at: null,
    staged_answer: {
      answer: 'approve',
      note: null,
      staged_at: new Date(Date.now() - 2 * 60_000).toISOString(),
      staged_via: 'voice',
    },
    ...overrides,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function buttonLabelled(text: string): HTMLButtonElement | undefined {
  return Array.from(container.querySelectorAll('button')).find(b =>
    (b.textContent ?? '').includes(text),
  );
}

describe('staged (spoken, uncommitted) verdict', () => {
  it('names the channel, the verdict and how long ago it was said', async () => {
    await act(async () => root.render(
      <DecisionItem decision={stagedDecision()} onAnswer={vi.fn()} onConflictSettled={() => {}} />,
    ));
    expect(container.textContent).toContain('Voice staged: Approve');
    expect(container.textContent).toContain('2m ago');
  });

  it('never claims the answer happened, and says what committing would do', async () => {
    await act(async () => root.render(
      <DecisionItem decision={stagedDecision()} onAnswer={vi.fn()} onConflictSettled={() => {}} />,
    ));
    expect(container.textContent).toContain('Heard, not committed');
    expect(container.textContent).toContain('nothing has happened yet');
    // The gated effect is spelled out, so the single tap is still informed.
    expect(container.textContent).toContain('may go ahead with this action');
  });

  it('commits exactly the staged verdict in one tap, through the ordinary answer path', async () => {
    const onAnswer = vi.fn().mockResolvedValue({ ok: true, decision: stagedDecision(), effect: null, effect_error: null });
    await act(async () => root.render(
      <DecisionItem decision={stagedDecision()} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));

    const commit = buttonLabelled('Commit approve');
    expect(commit).toBeTruthy();
    await act(async () => { commit!.click(); });

    // One tap — no second confirm step in between — and the verdict travels
    // verbatim. Attribution is the route's job (jesse + this device), which is
    // exactly why nothing here names the channel.
    expect(onAnswer).toHaveBeenCalledTimes(1);
    expect(onAnswer).toHaveBeenCalledWith('d-staged-1', { answer: 'approve', note: undefined });
  });

  it('carries the words said alongside the verdict into the answer note', async () => {
    const decision = stagedDecision({
      staged_answer: {
        answer: 'reject',
        note: 'not until the backup finishes',
        staged_at: new Date().toISOString(),
        staged_via: 'voice',
      },
    });
    const onAnswer = vi.fn().mockResolvedValue({ ok: true, decision, effect: null, effect_error: null });
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));
    expect(container.textContent).toContain('not until the backup finishes');

    await act(async () => { buttonLabelled('Commit reject')!.click(); });
    expect(onAnswer).toHaveBeenCalledWith('d-staged-1', {
      answer: 'reject',
      note: 'not until the backup finishes',
    });
  });

  it('offers a discard that is as reachable as the commit, and answers nothing', async () => {
    const onAnswer = vi.fn();
    const onDiscardStaged = vi.fn().mockResolvedValue(undefined);
    await act(async () => root.render(
      <DecisionItem
        decision={stagedDecision()}
        onAnswer={onAnswer}
        onConflictSettled={() => {}}
        onDiscardStaged={onDiscardStaged}
      />,
    ));

    const discard = buttonLabelled('Discard');
    expect(discard).toBeTruthy();
    await act(async () => { discard!.click(); });
    expect(onDiscardStaged).toHaveBeenCalledWith('d-staged-1');
    expect(onAnswer).not.toHaveBeenCalled();
  });

  it('shows nothing at all when no verdict is staged — the ordinary row is unchanged', async () => {
    const plain = stagedDecision({ staged_answer: null });
    await act(async () => root.render(
      <DecisionItem decision={plain} onAnswer={vi.fn()} onConflictSettled={() => {}} />,
    ));
    expect(container.querySelector('[data-testid="staged-d-staged-1"]')).toBeNull();
    expect(container.textContent).not.toContain('Heard, not committed');
    // The normal Approve/Reject affordances are still the way in.
    expect(buttonLabelled('Approve')).toBeTruthy();
    expect(buttonLabelled('Reject')).toBeTruthy();
  });
});
