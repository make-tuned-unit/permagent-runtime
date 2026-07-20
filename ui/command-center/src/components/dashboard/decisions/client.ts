/**
 * Decision Inbox — real daemon client (Lane L4, default since the real-API
 * swap). The mock stays behind VITE_DECISIONS_MOCK=1 for development.
 *
 * Auth: Bearer token via loadDaemonToken() from lib/api.ts — the proven,
 * cached path every other surface uses (never a separate IPC invoke).
 *
 * Wire mapping (cited to the Rust handlers):
 *  - GET  /api/decisions[?all=1] → { items, summary }
 *    (routes/decisions.rs:46-51, 101-114) — flattened here into
 *    DecisionsResponse for the UI.
 *  - POST /api/decisions/{id}/answer with camelCase body
 *    { answer, note?, choiceId?, inputText? } (routes/decisions.rs:53-60);
 *    response { decision, effect, effectError } (routes/decisions.rs:62-71).
 *    409 = already resolved (routes/decisions.rs:89-97).
 *  - GET  /api/decisions/history → { items, nextBefore }
 *    (routes/decisions.rs:79-85, 154-164).
 *  - Evidence digest: GET /api/projects/{pid}/cards/{gid} →
 *    metadataJson.dispatch_evidence.verdict.evidence_digest — the ONE
 *    canonical record (#466/#458; legacy metadataJson.verification read only
 *    as a fallback for pre-consolidation goals). CardResponse camelCase,
 *    routes/cards.rs:57-73; digest schema verification/digest.rs:89-107.
 */

import { getApiBaseUrl, loadDaemonToken } from '../../../lib/api';
import type {
  AnswerBody,
  AnswerOutcome,
  Decision,
  DecisionHistoryResponse,
  DecisionsClient,
  DecisionsResponse,
  DispatchEvidenceData,
  EvidenceDigestData,
  HistoryItem,
  InboxSummary,
} from './types';
import { DecisionConflictError } from './types';

// ── Wire envelopes (daemon-side shapes before UI flattening) ────────────────

interface WireInboxResponse {
  items: Decision[];
  summary: InboxSummary;
}

interface WireAnswerResponse {
  decision: Decision;
  effect: string | null;
  effectError: string | null;
}

interface WireHistoryResponse {
  items: HistoryItem[];
  nextBefore: number | null;
}

/** apiFetch-alike that surfaces HTTP 409 as DecisionConflictError. */
async function decisionsFetch<T>(endpoint: string, options?: RequestInit): Promise<T> {
  const token = await loadDaemonToken();
  const headers: Record<string, string> = {
    'Content-Type': 'application/json',
    ...((options?.headers as Record<string, string>) ?? {}),
  };
  if (token) headers['Authorization'] = `Bearer ${token}`;
  const response = await fetch(`${getApiBaseUrl()}${endpoint}`, { ...options, headers });
  if (response.status === 409) throw new DecisionConflictError();
  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: 'Unknown error' }));
    throw new Error(error.message || error.error || `HTTP ${response.status}`);
  }
  return response.json() as Promise<T>;
}

export const realDecisionsClient: DecisionsClient = {
  async list(opts?: { all?: boolean }): Promise<DecisionsResponse> {
    const { items, summary } = await decisionsFetch<WireInboxResponse>(
      `/api/decisions${opts?.all ? '?all=1' : ''}`,
    );
    return {
      decisions: items,
      total_pending: summary.total_pending,
      handled_count: summary.handled_count,
      goals_in_flight: summary.goals_in_flight,
      oldest_pending_at: summary.oldest_pending_at,
    };
  },

  async answer(id: string, body: AnswerBody): Promise<AnswerOutcome> {
    // Wire body is camelCase (AnswerRequest rename_all, routes/decisions.rs:53-60).
    const wireBody: Record<string, unknown> = { answer: body.answer };
    if (body.note !== undefined) wireBody.note = body.note;
    if (body.choice_id !== undefined) wireBody.choiceId = body.choice_id;
    if (body.input_text !== undefined) wireBody.inputText = body.input_text;
    const res = await decisionsFetch<WireAnswerResponse>(
      `/api/decisions/${encodeURIComponent(id)}/answer`,
      { method: 'POST', body: JSON.stringify(wireBody) },
    );
    return { decision: res.decision, effect: res.effect, effect_error: res.effectError };
  },

  async history(): Promise<DecisionHistoryResponse> {
    const res = await decisionsFetch<WireHistoryResponse>('/api/decisions/history');
    return { items: res.items, next_before: res.nextBefore };
  },

  async evidence(projectId: string, goalId: string): Promise<EvidenceDigestData | null> {
    try {
      const card = await decisionsFetch<{ metadataJson?: Record<string, unknown> | null }>(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}`,
      );
      // #466 single-source: the verifier appends its verdict onto
      // dispatch_evidence. The legacy top-level `verification` key is read as
      // a fallback so goals verified before the consolidation still render.
      const evidence = card.metadataJson?.dispatch_evidence as
        | { verdict?: { evidence_digest?: EvidenceDigestData } }
        | undefined;
      const legacy = card.metadataJson?.verification as
        | { evidence_digest?: EvidenceDigestData }
        | undefined;
      const digest = evidence?.verdict?.evidence_digest ?? legacy?.evidence_digest;
      // Minimal shape check before handing to the renderer.
      if (digest && digest.checks_summary && typeof digest.verifier_summary === 'string') {
        return digest;
      }
      return null;
    } catch {
      return null; // missing card / no verification yet — evidence simply absent
    }
  },

  async dispatchEvidence(projectId: string, goalId: string): Promise<DispatchEvidenceData | null> {
    try {
      const card = await decisionsFetch<{ metadataJson?: Record<string, unknown> | null }>(
        `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}`,
      );
      const ev = card.metadataJson?.dispatch_evidence as DispatchEvidenceData | undefined;
      // Minimal shape check before handing to the renderer.
      if (ev && Array.isArray(ev.commits) && typeof ev.diffstat === 'string') {
        return ev;
      }
      return null;
    } catch {
      return null; // missing card / no dispatch evidence — simply absent
    }
  },

  async cancelGoal(projectId: string, goalId: string): Promise<void> {
    // #490: kills the worker if running and moves the goal to Cancelled. The
    // backend supersedes this (and any other) open decision for the goal.
    await decisionsFetch(
      `/api/projects/${encodeURIComponent(projectId)}/cards/${encodeURIComponent(goalId)}/cancel`,
      { method: 'POST' },
    );
  },
};

import { mockDecisionsClient } from './mockDecisions';

const useMock = import.meta.env.VITE_DECISIONS_MOCK === '1';

export const decisionsClient: DecisionsClient = useMock
  ? mockDecisionsClient
  : realDecisionsClient;
