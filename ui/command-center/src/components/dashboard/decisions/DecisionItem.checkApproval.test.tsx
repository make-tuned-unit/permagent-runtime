/** @vitest-environment jsdom */
/**
 * Tier-2 command approval card (verification approval ladder): a `choice`
 * decision with `payload.proposal === 'check_approval'` must render the
 * exact blocked command and its cwd — the whole point of the card is that
 * the user can see EXACTLY what they'd be authorising.
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

const command = 'rm -rf node_modules && cargo xtask verify --release --no-cache';
const cwd = '/Users/j/Documents/dev/permagent-runtime/crates/goose-server';

const decision: Decision = {
  id: 'd-check-1',
  kind: 'choice',
  goal_id: null,
  project_id: 'proj-1',
  tier: 2,
  headline: 'Run this command for a verification check?',
  detail: '',
  payload: {
    question: 'Run this command for a verification check?',
    options: [
      { id: 'approve-once', label: 'Approve once' },
      { id: 'approve-and-allowlist', label: 'Approve and allowlist "rm"' },
      { id: 'deny', label: 'Deny' },
    ],
    default: null,
    proposal: 'check_approval',
    check_approval: {
      command,
      cwd,
      first_token: 'rm',
      reason: 'Shell commands run by verification checks need your say-so.',
      tier: 'user',
      project_id: 'proj-1',
    },
  },
  rank: null,
  status: 'open',
  answer: null,
  answer_note: null,
  answer_choice_id: null,
  answer_input: null,
  acted_by: null,
  created_at: '2026-08-24T00:00:00Z',
  resolved_at: null,
};

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

describe('command approval Inbox card', () => {
  it('renders the exact command text and the cwd', async () => {
    const onAnswer = vi.fn();
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));

    expect(container.textContent).toContain(command);
    expect(container.textContent).toContain(cwd);
  });

  it('renders the command as plain text (no HTML interpretation) and shows the reason', async () => {
    const onAnswer = vi.fn();
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));

    const pre = container.querySelector('pre');
    expect(pre).toBeTruthy();
    expect(pre!.textContent).toBe(command);
    expect(pre!.innerHTML).not.toContain('<script');
    expect(container.textContent).toContain('Shell commands run by verification checks need your say-so.');
  });

  it('badges the card as a command approval', async () => {
    const onAnswer = vi.fn();
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));
    expect(container.textContent).toContain('command approval');
  });

  it('offers the exact three options from payload.options, including allowlist since first_token is present', async () => {
    const onAnswer = vi.fn();
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));
    const labels = Array.from(container.querySelectorAll('button')).map(b => b.textContent);
    expect(labels).toContain('Approve once');
    expect(labels).toContain('Approve and allowlist "rm"');
    expect(labels).toContain('Deny');
  });
});
