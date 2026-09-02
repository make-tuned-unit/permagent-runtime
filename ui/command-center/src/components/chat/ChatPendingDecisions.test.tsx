/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Decision } from '../dashboard/decisions/types';

const { useDecisions } = vi.hoisted(() => ({
  useDecisions: vi.fn(),
}));

vi.mock('../dashboard/decisions/useDecisions', () => ({ useDecisions }));
// The dock now renders the CANONICAL row (J3), so its collaborators are the
// canonical row's: the store seam it deep-links through, the persona name and
// the evidence client. Same mocks the Inbox's own row tests use.
vi.mock('../../lib/store', () => ({
  useCommandCenter: (sel: (s: { discussDecision: () => void; openGoalDetail: () => void }) => unknown) =>
    sel({ discussDecision: () => {}, openGoalDetail: () => {} }),
}));
vi.mock('../../lib/notifications', () => ({ toast: () => undefined }));
vi.mock('../settings/useSettings', () => ({ usePersona: () => ({ data: null }) }));
vi.mock('../dashboard/decisions/client', () => ({ decisionsClient: {} }));

import { ChatPendingDecisions } from './ChatPendingDecisions';

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

function openDecision(partial: Partial<Decision> & Pick<Decision, 'id' | 'kind' | 'headline'>): Decision {
  return {
    goal_id: null,
    project_id: null,
    tier: 2,
    detail: '',
    payload: null,
    rank: null,
    status: 'open',
    answer: null,
    answer_note: null,
    answer_choice_id: null,
    answer_input: null,
    acted_by: null,
    created_at: '2026-08-21T00:00:00Z',
    resolved_at: null,
    ...partial,
  };
}

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useDecisions.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ChatPendingDecisions', () => {
  it('renders nothing when the inbox is empty', async () => {
    useDecisions.mockReturnValue({
      data: { decisions: [] },
      answer: vi.fn(),
    });
    await act(async () => root.render(<ChatPendingDecisions />));
    expect(container.querySelector('[data-testid="chat-pending-decisions"]')).toBeNull();
  });

  it('approves a pending decision from chat without opening the inbox', async () => {
    const answer = vi.fn(async () => ({ ok: true }));
    useDecisions.mockReturnValue({
      data: {
        decisions: [
          openDecision({
            id: 'd-enrich',
            kind: 'enrichment_proposal',
            headline: 'Save details for Jane Doe',
          }),
        ],
      },
      answer,
      refresh: vi.fn(),
    });
    await act(async () => root.render(<ChatPendingDecisions />));
    expect(container.textContent).toContain('Save details for Jane Doe');
    const click = (label: string) => {
      const btn = [...container.querySelectorAll('button')].find(b => b.textContent === label);
      expect(btn, `expected a "${label}" control`).toBeTruthy();
      return act(async () => btn!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    };
    // The canonical row's contract, now shared: an answer is confirmed inline,
    // one item at a time. The dock used to confirm only three kinds and fire
    // straight through on the rest.
    await click('Approve');
    await click('Confirm approve');
    expect(answer).toHaveBeenCalledWith('d-enrich', { answer: 'approve', note: undefined });
  });

  it('renders the canonical row, so chat and the Inbox answer the same way', async () => {
    useDecisions.mockReturnValue({
      data: {
        decisions: [openDecision({ id: 'd-1', kind: 'risk_gate', headline: 'Allow the push?' })],
      },
      answer: vi.fn(),
      refresh: vi.fn(),
    });
    await act(async () => root.render(<ChatPendingDecisions />));
    // `data-testid="decision-<id>"` is DecisionItem's own marker: the dock is
    // no longer drawing a decision card of its own.
    expect(container.querySelector('[data-testid="decision-d-1"]')).toBeTruthy();
  });
});
