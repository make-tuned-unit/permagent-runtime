/**
 * Decision Inbox — client selection (Lane L4).
 *
 * Real client implements Lane L1's contract; the mock server is selected by
 * VITE_DECISIONS_MOCK=1. Both sit behind the DecisionsClient seam so the swap
 * is a one-line change at integration time.
 */

import { getApiBaseUrl, loadDaemonToken } from '../../../lib/api';
import type {
  AnswerBody,
  Decision,
  DecisionHistoryResponse,
  DecisionsClient,
  DecisionsResponse,
} from './types';
import { DecisionConflictError } from './types';

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

const realDecisionsClient: DecisionsClient = {
  list: (opts?: { all?: boolean }) =>
    decisionsFetch<DecisionsResponse>(`/api/decisions${opts?.all ? '?all=1' : ''}`),
  answer: (id: string, body: AnswerBody) =>
    decisionsFetch<Decision>(`/api/decisions/${encodeURIComponent(id)}/answer`, {
      method: 'POST',
      body: JSON.stringify(body),
    }),
  history: () => decisionsFetch<DecisionHistoryResponse>('/api/decisions/history'),
};

import { mockDecisionsClient } from './mockDecisions';

const useMock = import.meta.env.VITE_DECISIONS_MOCK === '1';

export const decisionsClient: DecisionsClient = useMock
  ? mockDecisionsClient
  : realDecisionsClient;
