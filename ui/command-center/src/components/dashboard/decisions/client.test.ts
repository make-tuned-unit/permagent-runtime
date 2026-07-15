/**
 * Decision Inbox — real-client wire-contract tests (Lane L4).
 *
 * Pins the client to L1's ACTUAL wire format with a stubbed fetch:
 * envelope flattening, camelCase answer body, Bearer auth, 409 mapping,
 * history cursor, and the card-sourced L2 evidence digest path.
 * Fixture shapes mirror routes/decisions.rs + verification/digest.rs.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('../../../lib/api', () => ({
  getApiBaseUrl: () => 'http://daemon.test',
  loadDaemonToken: async () => 'tok-123',
}));

import { realDecisionsClient } from './client';
import { DecisionConflictError, choiceOptions, draftText, recommendedChoiceId, resolutionText } from './types';
import type { Decision } from './types';

const wireDecision = {
  id: 'd-1',
  kind: 'approve_review',
  goal_id: 'g-1',
  project_id: 'p-1',
  tier: 2,
  headline: 'Ship the change: decision queue saving',
  detail: 'Worker reported success; goal moved to Review.',
  payload: {},
  rank: 0.9,
  status: 'open',
  answer: null,
  answer_note: null,
  answer_choice_id: null,
  answer_input: null,
  acted_by: null,
  created_at: '2026-06-12T00:00:00Z',
  resolved_at: null,
  goal_title: 'Decision queue saving',
};

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

let fetchMock: ReturnType<typeof vi.fn>;

beforeEach(() => {
  fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe('realDecisionsClient.list', () => {
  it('flattens the {items, summary} envelope and sends the Bearer token', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      items: [wireDecision],
      summary: {
        total_pending: 13,
        handled_count: 7,
        goals_in_flight: 6,
        oldest_pending_at: '2026-06-11T20:00:00Z',
      },
    }));

    const res = await realDecisionsClient.list();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://daemon.test/api/decisions');
    expect((init.headers as Record<string, string>).Authorization).toBe('Bearer tok-123');

    expect(res.decisions).toHaveLength(1);
    expect(res.decisions[0].headline).toBe('Ship the change: decision queue saving');
    expect(res.decisions[0].goal_title).toBe('Decision queue saving');
    expect(res.total_pending).toBe(13);
    expect(res.handled_count).toBe(7);
    expect(res.goals_in_flight).toBe(6);
    expect(res.oldest_pending_at).toBe('2026-06-11T20:00:00Z');
  });

  it('passes ?all=1 for the overflow expansion', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      items: [],
      summary: { total_pending: 0, handled_count: 0, goals_in_flight: 0, oldest_pending_at: null },
    }));
    await realDecisionsClient.list({ all: true });
    expect(fetchMock.mock.calls[0][0]).toBe('http://daemon.test/api/decisions?all=1');
  });
});

describe('realDecisionsClient.answer', () => {
  it('sends the camelCase wire body (choiceId/inputText) and unwraps the envelope', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      decision: { ...wireDecision, status: 'answered', answer: 'choice', answer_choice_id: 'opt-a' },
      effect: null,
      effectError: null,
    }));

    const res = await realDecisionsClient.answer('d-1', {
      answer: 'choice',
      choice_id: 'opt-a',
      input_text: 'free text',
      note: 'a note',
    });

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://daemon.test/api/decisions/d-1/answer');
    expect(init.method).toBe('POST');
    const body = JSON.parse(init.body as string);
    // AnswerRequest is rename_all = camelCase (routes/decisions.rs:53-60).
    expect(body).toEqual({ answer: 'choice', note: 'a note', choiceId: 'opt-a', inputText: 'free text' });
    expect(body.choice_id).toBeUndefined();
    expect(body.input_text).toBeUndefined();

    expect(res.decision.answer).toBe('choice');
    expect(res.effect).toBeNull();
    expect(res.effect_error).toBeNull();
  });

  it('sends approve-with-edits as answer=edit + inputText to the mounted endpoint', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      decision: {
        ...wireDecision, kind: 'automation_proposal',
        status: 'answered', answer: 'edit', answer_input: 'git status && git pull --rebase',
      },
      effect: 'automation proposal approved with edits',
      effectError: null,
    }));

    const res = await realDecisionsClient.answer('d-1', {
      answer: 'edit',
      input_text: 'git status && git pull --rebase',
    });

    const [url, init] = fetchMock.mock.calls[0];
    expect(url).toBe('http://daemon.test/api/decisions/d-1/answer');
    expect(init.method).toBe('POST');
    const body = JSON.parse(init.body as string);
    // The revised draft rides in inputText (AnswerRequest camelCase); the daemon
    // stores answer='edit' and learns the correction.
    expect(body).toEqual({ answer: 'edit', inputText: 'git status && git pull --rebase' });
    expect(res.decision.answer).toBe('edit');
    expect(res.effect).toBe('automation proposal approved with edits');
  });

  it('surfaces the gated effect and maps effectError → effect_error', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      decision: { ...wireDecision, status: 'answered', answer: 'approve' },
      effect: 'goal approved: Review → Complete',
      effectError: 'dependents promotion failed',
    }));
    const res = await realDecisionsClient.answer('d-1', { answer: 'approve' });
    expect(res.effect).toBe('goal approved: Review → Complete');
    expect(res.effect_error).toBe('dependents promotion failed');
  });

  it('maps HTTP 409 to DecisionConflictError', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse('Decision already resolved (status: answered)', 409));
    await expect(realDecisionsClient.answer('d-1', { answer: 'approve' }))
      .rejects.toBeInstanceOf(DecisionConflictError);
  });
});

describe('realDecisionsClient.history', () => {
  it('maps {items, nextBefore} with the audit join', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({
      items: [{
        ...wireDecision,
        status: 'answered',
        answer: 'approve',
        acted_by: 'henry-policy',
        resolved_at: '2026-06-12T01:00:00Z',
        seq: 41,
        outcome: 'approve',
        audit_acted_by: 'henry-policy',
        audit_created_at: '2026-06-12T01:00:00Z',
      }],
      nextBefore: 41,
    }));
    const res = await realDecisionsClient.history();
    expect(res.items[0].seq).toBe(41);
    expect(res.items[0].audit_acted_by).toBe('henry-policy');
    expect(res.next_before).toBe(41);
  });
});

describe('realDecisionsClient.evidence', () => {
  it('reads metadataJson.verification.evidence_digest from the goal card', async () => {
    const digest = {
      version: 1,
      goal_id: 'g-1',
      goal_title: 'Decision queue saving',
      checks_summary: { passed_count: 4, total_count: 4, tests_run: 412, one_line: 'All 4 automated checks passed (412 tests)' },
      verifier_summary: "Henry's reviewer confirmed the work matches what was approved",
      costs: { cost_usd: 4.02, accumulated_total_tokens: 1900000, worker_session_id: 's-1', attempt_count: 1, note: null },
      checks: [],
      diff: { files_changed: 14, insertions: 482, deletions: 97, per_file: [] },
      out_of_path_files: [],
      verifier: { status: 'pass', rationale: 'Looks right.', model: 'qwen2.5:7b', degraded_reason: null },
      started_at: 't0',
      finished_at: 't1',
    };
    // CardResponse is camelCase (routes/cards.rs:57-73) → metadataJson.
    fetchMock.mockResolvedValueOnce(jsonResponse({
      id: 'g-1', projectId: 'p-1', metadataJson: { verification: { evidence_digest: digest } },
    }));
    const res = await realDecisionsClient.evidence('p-1', 'g-1');
    expect(fetchMock.mock.calls[0][0]).toBe('http://daemon.test/api/projects/p-1/cards/g-1');
    expect(res?.checks_summary.one_line).toBe('All 4 automated checks passed (412 tests)');
    expect(res?.costs.cost_usd).toBe(4.02);
  });

  it('returns null when no verification has been recorded', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ id: 'g-1', projectId: 'p-1', metadataJson: {} }));
    expect(await realDecisionsClient.evidence('p-1', 'g-1')).toBeNull();
  });

  it('returns null on fetch failure (missing card)', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ message: 'not found' }, 404));
    expect(await realDecisionsClient.evidence('p-1', 'g-1')).toBeNull();
  });
});

describe('payload helpers', () => {
  const choice: Decision = {
    ...wireDecision,
    kind: 'choice',
    payload: {
      question: 'Pick one',
      options: [{ id: 'a', label: 'Option A' }, { id: 'b', label: 'Option B' }],
      default: 'a',
    },
  };

  it('choiceOptions reads payload.options for choice decisions only', () => {
    expect(choiceOptions(choice).map(o => o.id)).toEqual(['a', 'b']);
    expect(choiceOptions({ ...wireDecision } as Decision)).toEqual([]);
  });

  it('recommendedChoiceId reads payload.default', () => {
    expect(recommendedChoiceId(choice)).toBe('a');
    expect(recommendedChoiceId({ ...choice, payload: { question: 'q', options: [] } })).toBeNull();
  });

  it('resolutionText maps the answer enum without inventing content', () => {
    expect(resolutionText({ ...wireDecision, answer: 'approve', acted_by: 'henry-policy' } as Decision))
      .toBe('Approved by Aria');
    expect(resolutionText({ ...wireDecision, answer: 'reject', acted_by: 'jesse', answer_note: 'not yet' } as Decision))
      .toBe('Rejected — note: not yet');
    expect(resolutionText({ ...wireDecision, answer: 'edit', acted_by: 'jesse' } as Decision))
      .toBe('Accepted with edits');
  });

  it('draftText reads payload.draft only when it is a non-empty string', () => {
    const withDraft: Decision = {
      ...wireDecision, kind: 'automation_proposal',
      payload: { normalized_command: 'ls', draft: 'ls -la' },
    };
    expect(draftText(withDraft)).toBe('ls -la');
    // Absent, empty, or non-string drafts read as null (no edit affordance).
    expect(draftText({ ...wireDecision } as Decision)).toBeNull();
    expect(draftText({ ...withDraft, payload: { draft: '   ' } } as Decision)).toBeNull();
    expect(draftText({ ...withDraft, payload: { draft: 42 } } as Decision)).toBeNull();
  });
});
