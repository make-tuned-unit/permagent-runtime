/** @vitest-environment jsdom */
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

import { DecisionActions } from './DecisionItem';
import { effectTextFor } from './decisionEffects';
import type { Decision } from './types';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function baseDecision(overrides: Partial<Decision>): Decision {
  return {
    id: 'd-1',
    kind: 'unblock',
    goal_id: 'g-1',
    project_id: 'p-1',
    tier: 2,
    headline: 'Unblock this goal',
    detail: 'Need one input',
    payload: { reason: 'stuck' },
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    created_at: '2026-09-22T00:00:00Z',
    resolved_at: null,
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

function click(text: string) {
  const button = Array.from(container.querySelectorAll('button')).find(
    b => (b.textContent ?? '').trim() === text || (b.textContent ?? '').includes(text),
  );
  expect(button).toBeTruthy();
  button!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
}

function setTextarea(textarea: HTMLTextAreaElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
  setter.call(textarea, value);
  textarea.dispatchEvent(new Event('input', { bubbles: true }));
}

async function waitFor(condition: () => boolean, timeoutMs = 2000) {
  const started = Date.now();
  while (!condition()) {
    if (Date.now() - started > timeoutMs) {
      throw new Error('Timed out waiting for condition');
    }
    await act(async () => {
      await new Promise(resolve => setTimeout(resolve, 25));
    });
  }
}

describe('DecisionActions', () => {
  it('uses scheduled-job unblock confirm copy, while goal unblock keeps the goal copy', async () => {
    const onAnswered = vi.fn().mockResolvedValue({ ok: true, decision: baseDecision({}), effect: null, effect_error: null });
    const scheduledDecision = baseDecision({
      kind: 'unblock',
      payload: {
        target: { type: 'scheduled_job', job_id: 'job-42' },
        reason: 'consecutive_failure_limit',
        pause_episode_id: 'ep-7',
        answers: [
          { id: 'approve', label: 'Resume schedule' },
          { id: 'reject', label: 'Keep paused' },
        ],
      },
    });

    await act(async () => root.render(
      <DecisionActions
        decision={scheduledDecision}
        agentName="Henry"
        note=""
        onAnswered={onAnswered}
        onConflict={() => {}}
      />,
    ));

    await act(async () => click('Approve'));
    expect(container.textContent).toContain('Confirm approve — resume the scheduled job "job-42" (episode ep-7); this resumes the job and resets consecutive failures and retry count to 0.');

    // A goal unblock has no Approve button (it takes a freeform instruction),
    // so its confirm copy is checked at the one source the confirm row reads.
    const goalUnblock = baseDecision({ kind: 'unblock', payload: { reason: 'stuck' } });
    expect(effectTextFor('unblock', 'approve', 'Henry', undefined, goalUnblock))
      .toBe('Confirm approve — Henry will wake this goal up and let it try again.');
  });

  it('submits note separately from inputText', async () => {
    const onAnswered = vi.fn().mockResolvedValue({ ok: true, decision: baseDecision({}), effect: null, effect_error: null });
    await act(async () => root.render(
      <DecisionActions
        decision={baseDecision({})}
        agentName="Henry"
        note=""
        onAnswered={onAnswered}
        onConflict={() => {}}
      />,
    ));

    await act(async () => click('Add note'));
    const noteArea = container.querySelector('textarea') as HTMLTextAreaElement;
    await act(async () => setTextarea(noteArea, 'Carry this with the answer'));

    await act(async () => click('Send answer'));
    const inputArea = Array.from(container.querySelectorAll('textarea')).find(
      t => (t.getAttribute('placeholder') ?? '').includes('goes to the worker exactly as written'),
    ) as HTMLTextAreaElement | undefined;
    expect(inputArea).toBeTruthy();
    await act(async () => setTextarea(inputArea!, 'Postgres 16'));
    await act(async () => click('Send answer'));
    await act(async () => click('Confirm answer'));

    expect(onAnswered).toHaveBeenCalledWith({
      answer: 'input',
      input_text: 'Postgres 16',
      note: 'Carry this with the answer',
    });
  });

  it('preserves edited draft intent on failure so retry sends the same input_text', async () => {
    const decision = baseDecision({
      kind: 'automation_proposal',
      payload: { draft: 'git status && git pull' },
    });
    const onAnswered = vi.fn()
      .mockRejectedValueOnce(new Error('network down'))
      .mockResolvedValueOnce({ ok: true, decision, effect: null, effect_error: null });

    await act(async () => root.render(
      <DecisionActions
        decision={decision}
        agentName="Henry"
        note=""
        onAnswered={onAnswered}
        onConflict={() => {}}
      />,
    ));

    await act(async () => click('Edit & accept'));
    const editArea = container.querySelector('textarea') as HTMLTextAreaElement;
    await act(async () => setTextarea(editArea, 'git pull --rebase --autostash'));
    await act(async () => click('Accept edited'));
    await act(async () => click('Confirm accept'));

    expect(container.textContent).toContain("Couldn't send");
    await waitFor(() => {
      const confirm = Array.from(container.querySelectorAll('button')).find(
        b => (b.textContent ?? '').includes('Confirm accept'),
      ) as HTMLButtonElement | undefined;
      return Boolean(confirm && !confirm.disabled);
    });
    await act(async () => click('Confirm accept'));
    expect(onAnswered).toHaveBeenNthCalledWith(1, {
      answer: 'edit',
      input_text: 'git pull --rebase --autostash',
      note: undefined,
    });
    expect(onAnswered).toHaveBeenNthCalledWith(2, {
      answer: 'edit',
      input_text: 'git pull --rebase --autostash',
      note: undefined,
    });
  });

  it('prevents duplicate submit while a confirm action is in flight', async () => {
    let resolveAnswer: ((v: { ok: true; decision: Decision; effect: null; effect_error: null }) => void) | undefined;
    const pending = new Promise<{ ok: true; decision: Decision; effect: null; effect_error: null }>(resolve => {
      resolveAnswer = resolve;
    });
    const decision = baseDecision({ kind: 'risk_gate', payload: { action_class: 'cc_shell' } });
    const onAnswered = vi.fn().mockReturnValue(pending);

    await act(async () => root.render(
      <DecisionActions
        decision={decision}
        agentName="Henry"
        note=""
        onAnswered={onAnswered}
        onConflict={() => {}}
      />,
    ));

    await act(async () => click('Approve'));
    const confirm = Array.from(container.querySelectorAll('button')).find(
      b => (b.textContent ?? '').includes('Confirm approve'),
    ) as HTMLButtonElement | undefined;
    expect(confirm).toBeTruthy();
    await act(async () => {
      confirm!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
      confirm!.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });
    expect(onAnswered).toHaveBeenCalledTimes(1);
    await act(async () => {
      resolveAnswer!({ ok: true, decision, effect: null, effect_error: null });
      await pending;
    });
  });
});
