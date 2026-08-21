/** @vitest-environment jsdom */
import { act } from 'react-dom/test-utils';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Decision } from '../dashboard/decisions/types';

const { useDecisions } = vi.hoisted(() => ({
  useDecisions: vi.fn(),
}));

vi.mock('../dashboard/decisions/useDecisions', () => ({ useDecisions }));

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
    });
    await act(async () => root.render(<ChatPendingDecisions />));
    expect(container.textContent).toContain('Save details for Jane Doe');
    const approve = [...container.querySelectorAll('button')].find(b => b.textContent === 'Approve');
    expect(approve).toBeTruthy();
    await act(async () => approve!.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(answer).toHaveBeenCalledWith('d-enrich', { answer: 'approve' });
  });
});
