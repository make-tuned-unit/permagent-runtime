/** @vitest-environment jsdom */
/**
 * Rejecting an enrichment proposal must offer a find-online hint field, and
 * that hint must travel as the answer note (the daemon writes it onto the person).
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

const decision: Decision = {
  id: 'd-enrich',
  kind: 'enrichment_proposal',
  goal_id: null,
  project_id: null,
  tier: 2,
  headline: 'Proposed enrichment for Example Person',
  detail: 'linkedin, company',
  payload: {
    person_name: 'Example Person',
    graph_entity_id: 'ab'.repeat(32),
    entity_uuid: 'uuid-ex',
    fields: [{ field_name: 'company', value: 'Example Co', source_url: 'https://example.com' }],
  },
  rank: null,
  status: 'open',
  answer: null,
  answer_note: null,
  answer_choice_id: null,
  answer_input: null,
  acted_by: null,
  created_at: '2026-08-20T00:00:00Z',
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

describe('enrichment reject find-online hint', () => {
  it('shows a find-online field on reject and sends it as the answer note', async () => {
    const onAnswer = vi.fn(async () => ({
      ok: true as const,
      decision,
      effect: null,
      effect_error: null,
    }));
    await act(async () => root.render(
      <DecisionItem decision={decision} onAnswer={onAnswer} onConflictSettled={() => {}} />,
    ));

    const reject = [...container.querySelectorAll('button')].find(b => b.textContent === 'Reject');
    expect(reject).toBeTruthy();
    await act(async () => reject!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    const hint = container.querySelector('textarea') as HTMLTextAreaElement | null;
    expect(hint).toBeTruthy();
    expect(hint!.placeholder).toMatch(/find this person online/i);

    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!;
    await act(async () => {
      setter.call(hint, 'Halifax coworking, director of sales');
      hint!.dispatchEvent(new Event('input', { bubbles: true }));
    });

    const confirm = [...container.querySelectorAll('button')].find(b => b.textContent === 'Confirm reject');
    expect(confirm).toBeTruthy();
    await act(async () => confirm!.dispatchEvent(new MouseEvent('click', { bubbles: true })));

    expect(onAnswer).toHaveBeenCalledWith('d-enrich', {
      answer: 'reject',
      note: 'Halifax coworking, director of sales',
    });
  });
});
